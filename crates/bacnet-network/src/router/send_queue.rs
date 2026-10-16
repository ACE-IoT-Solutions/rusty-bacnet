//! One port's bounded backlog. All four priority lanes share one capacity;
//! taking a send releases its slot before the transport starts that send.
//! Higher network priority wins, with arrival order within each lane.
//!
//! Admission, selection and closure share a short synchronous lock. No lock
//! crosses an await, and no queued send is evicted. The sole receiver uses
//! `Notify::notify_one`'s retained permit, so admission or the last sender's
//! drop between checking the queue and awaiting cannot lose a wakeup.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::Notify;

use super::SendRequest;

pub(super) const CAPACITY: usize = 256;

struct State {
    lanes: [VecDeque<SendRequest>; 4],
    len: usize,
    capacity: usize,
    senders: usize,
    receiver: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct Sender(Arc<Shared>);

pub(super) struct Receiver(Arc<Shared>);

pub(super) fn channel(capacity: usize) -> (Sender, Receiver) {
    assert!(capacity > 0);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            lanes: std::array::from_fn(|_| VecDeque::new()),
            len: 0,
            capacity,
            senders: 1,
            receiver: true,
        }),
        ready: Notify::new(),
    });
    (Sender(Arc::clone(&shared)), Receiver(shared))
}

impl Sender {
    /// Admit at the engine's validated network priority (0 through 3).
    /// Closure wins over fullness, as for the previous bounded channel.
    pub(super) fn try_send(
        &self,
        priority: u8,
        request: SendRequest,
    ) -> Result<(), TrySendError<SendRequest>> {
        {
            let mut state = self.0.lock();
            if !state.receiver {
                return Err(TrySendError::Closed(request));
            }
            if state.len == state.capacity {
                return Err(TrySendError::Full(request));
            }
            state.lanes[usize::from(priority)].push_back(request);
            state.len += 1;
        }
        self.0.ready.notify_one();
        Ok(())
    }
}

impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.lock().senders += 1;
        Self(Arc::clone(&self.0))
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let last = {
            let mut state = self.0.lock();
            state.senders -= 1;
            state.senders == 0
        };
        if last {
            self.0.ready.notify_one();
        }
    }
}

impl Receiver {
    pub(super) fn try_recv(&mut self) -> Result<SendRequest, TryRecvError> {
        let mut state = self.0.lock();
        for lane in state.lanes.iter_mut().rev() {
            if let Some(request) = lane.pop_front() {
                state.len -= 1;
                return Ok(request);
            }
        }
        Err(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }

    /// Cancellation while waiting consumes no send. Only this receiver can
    /// wait, and its next call checks the queue even after a notification.
    pub(super) async fn recv(&mut self) -> Option<SendRequest> {
        // Like Tokio's bounded receiver, cooperate even if the queue and
        // transport stay ready. Yield before selecting a send, not after
        // removing one that a higher-priority arrival could overtake.
        tokio::task::consume_budget().await;
        loop {
            match self.try_recv() {
                Ok(request) => return Some(request),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => self.0.ready.notified().await,
            }
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let discarded = {
            let mut state = self.0.lock();
            state.receiver = false;
            state.len = 0;
            std::mem::take(&mut state.lanes)
        };
        // Release payloads even while senders still own the shared state.
        drop(discarded);
    }
}

#[cfg(test)]
#[path = "send_queue_tests.rs"]
mod tests;
