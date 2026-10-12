//! This transport's own management requests (`read_bdt` and the other
//! helpers): one exchange with the node each, from admission to the
//! response, a timeout, a failed send or the caller giving up.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{ExchangeId, ExchangeRefusal, LocalFailure};
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::oneshot;

use super::node::{core_duration, NodeHandle, Outcome, Waiter};
use crate::bvll::{decode_bip_mac, encode_bvll, BvllMessage};

/// A manual exchange admitted by the node. Dropping it before it ends
/// gives it up, as when the caller stops waiting.
pub(super) struct Exchange {
    node: Arc<NodeHandle>,
    id: ExchangeId,
}

impl NodeHandle {
    /// Send `request` to the B/IP address `target` and wait up to `timeout`
    /// after the send for the response that answers it.
    pub(super) async fn request(
        self: &Arc<Self>,
        target: &[u8],
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<BvllMessage, Error> {
        let (ip, port) = decode_bip_mac(target)?;
        let peer = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        let (exchange, response) = self.begin(peer, request, timeout)?;
        let mut buf = BytesMut::new();
        if let Err(error) = encode_bvll(&mut buf, &request) {
            exchange.failed();
            return Err(error);
        }
        if let Err(error) = self.socket().send_to(&buf, peer).await {
            exchange.failed();
            return Err(Error::Transport(error));
        }
        exchange.sent();
        match response.await {
            Ok(outcome) => outcome,
            Err(_) => Err(Error::Encoding("BVLC response channel dropped".into())),
        }
    }

    /// Admit a manual exchange of `request` with `peer`.
    pub(super) fn begin(
        self: &Arc<Self>,
        peer: SocketAddrV4,
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<(Exchange, oneshot::Receiver<Outcome>), Error> {
        let (tx, rx) = oneshot::channel();
        let id = self.with(|state, now| {
            if state.stopped {
                return Err(Error::Encoding("B/IP management client is stopped".into()));
            }
            let (id, _) = state
                .node
                .begin_exchange(peer, request, core_duration(timeout), now)
                .map_err(refusal_error)?;
            let waiter = Waiter {
                tx,
                timeout,
                early: None,
            };
            state.waiters.insert(id, waiter);
            Ok(id)
        })?;
        let exchange = Exchange {
            node: Arc::clone(self),
            id,
        };
        Ok((exchange, rx))
    }
}

impl Exchange {
    /// The request went out: its timeout starts, and a response that beat
    /// the send is handed over now.
    pub(super) fn sent(&self) {
        self.node.with(|state, now| {
            if !state.node.exchange_sent(self.id, now) {
                return;
            }
            let early = state
                .waiters
                .get_mut(&self.id)
                .and_then(|waiter| waiter.early.take());
            let outcome =
                early.unwrap_or_else(|| Err(Error::Encoding("BVLC response lost".into())));
            state.complete(self.id, outcome);
        });
    }

    /// Encoding or sending the request failed.
    pub(super) fn failed(&self) {
        self.end(LocalFailure::SendFailed);
    }

    fn end(&self, why: LocalFailure) {
        self.node.with(|state, now| {
            state.waiters.remove(&self.id);
            state.node.exchange_failed(self.id, why, now);
        });
    }
}

impl Drop for Exchange {
    /// Give the exchange up, unless it already ended.
    fn drop(&mut self) {
        self.end(LocalFailure::Cancelled);
    }
}

fn refusal_error(refusal: ExchangeRefusal) -> Error {
    match refusal {
        ExchangeRefusal::SameExchange | ExchangeRefusal::ResultClass | ExchangeRefusal::Full => {
            Error::Encoding("BVLC management request already in flight".into())
        }
        ExchangeRefusal::ManagedRegistration | ExchangeRefusal::NotARequest => {
            Error::Encoding(format!("BVLC management request refused: {refusal}"))
        }
    }
}

/// The error for a response of the wrong function.
pub(super) fn unexpected_response(expected: BvlcFunction, got: BvlcFunction) -> Error {
    Error::Encoding(format!("expected BVLC response {expected:?}, got {got:?}"))
}

/// The result code of a BVLC-Result, or why `msg` isn't a well-formed one.
pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    if msg.function != BvlcFunction::BVLC_RESULT {
        return Err(unexpected_response(BvlcFunction::BVLC_RESULT, msg.function));
    }
    match msg.message()? {
        Bvll::BvlcResult(code) => Ok(code),
        other => Err(unexpected_response(
            BvlcFunction::BVLC_RESULT,
            other.function(),
        )),
    }
}
