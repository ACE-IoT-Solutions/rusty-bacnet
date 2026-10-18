//! Weak application reply authority for one running router.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

use bacnet_router_core::router::ReplyPlan;
use bacnet_transport::port::{DirectResponseScope, ResponseScopeGuard};
use bacnet_types::{error::Error, MacAddr};
use bytes::Bytes;
use tokio::sync::oneshot;

use super::{dispatch::Outputs, SendRequest};

#[derive(Debug, Default)]
pub(crate) struct ReplyLifetime {
    stopped: AtomicBool,
    pub(crate) direct: DirectResponseScope,
}

impl ReplyLifetime {
    pub(super) fn seal(&self) {
        self.stopped.store(true, Ordering::Release);
        self.direct.seal();
    }
    fn check(&self) -> Result<(), Error> {
        if self.stopped.load(Ordering::Acquire) {
            Err(unavailable())
        } else {
            Ok(())
        }
    }
}

/// Saved router endpoint and return path. It owns no router, queue or socket.
/// Only local delivery by a running router creates this capability.
#[derive(Clone)]
pub struct RouterResponse {
    pub(crate) plan: ReplyPlan,
    pub(crate) max_npdu: u16,
    outputs: Weak<Outputs>,
    lifetime: Weak<ReplyLifetime>,
}

impl std::fmt::Debug for RouterResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterResponse")
            .field("plan", &self.plan)
            .finish_non_exhaustive()
    }
}

impl RouterResponse {
    pub(super) fn new(plan: ReplyPlan, max_npdu: u16, outputs: &Arc<Outputs>) -> Self {
        Self {
            plan,
            max_npdu,
            outputs: Arc::downgrade(outputs),
            lifetime: Arc::downgrade(&outputs.replies),
        }
    }
    pub(crate) fn same_origin(&self, other: &Self) -> bool {
        self.plan == other.plan && self.lifetime.ptr_eq(&other.lifetime)
    }
    pub(crate) fn check(&self) -> Result<(), Error> {
        self.lifetime()?.check()
    }
    pub(crate) fn lifetime(&self) -> Result<Arc<ReplyLifetime>, Error> {
        let lifetime = self.lifetime.upgrade().ok_or_else(unavailable)?;
        lifetime.check()?;
        Ok(lifetime)
    }
    pub(crate) async fn send(
        &self,
        npdu: Bytes,
        priority: u8,
        owner: ResponseScopeGuard,
        on_issuance: impl FnOnce() + Send + 'static,
    ) -> Result<(), Error> {
        self.check()?;
        if !owner.is_open() {
            return Err(unavailable());
        }
        let (done, completed) = oneshot::channel();
        {
            let outputs = self.outputs.upgrade().ok_or_else(unavailable)?;
            let queue = outputs
                .send_txs
                .get(usize::from(self.plan.port().0))
                .ok_or_else(unavailable)?;
            queue
                .try_send(
                    priority,
                    SendRequest::Unicast {
                        npdu,
                        mac: MacAddr::from_slice(self.plan.peer()),
                        data_attributes: vec![],
                        reject_origin: None,
                        response: Some(ReplyCompletion {
                            done,
                            owner,
                            lifetime: self.lifetime.clone(),
                            on_issuance: Box::new(on_issuance),
                        }),
                    },
                )
                .map_err(|_| unavailable())?;
        }
        completed.await.map_err(|_| unavailable())?
    }
}

pub(super) struct ReplyCompletion {
    done: oneshot::Sender<Result<(), Error>>,
    owner: ResponseScopeGuard,
    lifetime: Weak<ReplyLifetime>,
    on_issuance: Box<dyn FnOnce() + Send>,
}

impl std::fmt::Debug for ReplyCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyCompletion").finish_non_exhaustive()
    }
}

impl ReplyCompletion {
    /// Cancellation or owner retirement before selection emits no bytes.
    pub(super) fn start(self) -> Option<oneshot::Sender<Result<(), Error>>> {
        if !self.owner.is_open()
            || self.done.is_closed()
            || self
                .lifetime
                .upgrade()
                .is_none_or(|lifetime| lifetime.check().is_err())
        {
            let _ = self.done.send(Err(unavailable()));
            return None;
        }
        (self.on_issuance)();
        Some(self.done)
    }
}

fn unavailable() -> Error {
    Error::Transport(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "original router response route unavailable",
    ))
}
