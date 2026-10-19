//! Carrying out what the engine asks to send (#1679).
//!
//! After every call into the engine, [`drain`] polls it until it is idle
//! and puts each send on its port's queue with `try_send`, under the
//! engine's lock: nothing here waits. A send an arrival prompted carries
//! that arrival's data attributes (RB-03); the startup announcements carry
//! none. A full or closed queue drops the send, logs the warning its kind
//! logged before the engine, and tells the engine, which ends a chunked
//! Initialize-Routing-Table-Ack answer there.
//!
//! With the engine's congestion policy on (#1697), each admission or
//! refusal also reports that port's backlog to the engine, and marks the
//! queue watched while the engine counts the port congested, so its sender
//! reports the backlog as it drains ([`report_backlog`]).

use bacnet_npdu::NetworkMessageType;
use bacnet_router_core::router::{Action, Cause, Dest, Kind, SendRefusal};
use bacnet_router_core::{Instant, PortId};
use bacnet_transport::port::DataAttribute;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

use super::engine::HostedRouter;
use super::send_queue;
use super::SendRequest;

/// Send what `router` asks for on the port queues `send_txs`, by port
/// index, until it is idle. `attributes` are the data attributes of the
/// arrival that prompted the sends, if any; `now` is the engine's time.
pub(super) fn drain(
    router: &mut HostedRouter,
    attributes: &[DataAttribute],
    send_txs: &[send_queue::Sender],
    now: Instant,
) {
    let congestion = router.options().congestion.is_some();
    // A fan-out polls the same NPDU once per port: share its buffer.
    let mut last: Option<Bytes> = None;
    loop {
        let Action::Send(send) = router.poll_action() else {
            return;
        };
        let npdu = match last.take() {
            Some(previous) if previous.as_ref() == send.npdu => previous,
            _ => Bytes::copy_from_slice(send.npdu),
        };
        last = Some(npdu.clone());
        let data_attributes = match send.cause {
            Cause::Ingress => attributes.to_vec(),
            Cause::Local => Vec::new(),
        };
        let request = match send.to {
            Dest::Unicast(mac) => SendRequest::Unicast {
                npdu,
                mac: MacAddr::from_slice(mac),
                data_attributes,
                response: None,
                reject_origin: send.reject_origin,
            },
            Dest::Broadcast => SendRequest::Broadcast {
                npdu,
                data_attributes,
                reject_origin: send.reject_origin,
            },
        };
        let (port, kind) = (send.port, send.kind);
        let tx = send_txs.get(usize::from(port.0));
        let result = match tx {
            Some(tx) => tx.try_send(send.priority, request),
            None => Err(TrySendError::Closed(())),
        };
        if let (true, Some(tx)) = (congestion, tx) {
            router.egress_backlog(port, tx.backlog(), now);
            tx.set_watched(router.port_congested(port));
        }
        if let Err(error) = result {
            refused(kind, &error);
            router.refused(SendRefusal::Full);
        }
    }
}

/// Report port `port`'s backlog, read from its receiver after a send was
/// taken, while the engine counts the port congested; then carry out the
/// Router-Available a drained port may have queued. Synchronous: the
/// engine's lock is held for the report and the drain only.
pub(super) fn report_backlog(
    shared: &super::dispatch::Shared,
    outputs: &std::sync::Weak<super::dispatch::Outputs>,
    port: PortId,
    queue: &send_queue::Receiver,
) {
    let Some(backlog) = queue.watched_backlog() else {
        return;
    };
    let outputs = outputs.upgrade();
    let send_txs = outputs
        .as_ref()
        .map_or(&[][..], |outputs| outputs.send_txs.as_slice());
    let now = shared.engine.now();
    let mut router = shared.engine.lock();
    router.egress_backlog(port, backlog, now);
    queue.set_watched(router.port_congested(port));
    drain(&mut router, &[], send_txs, now);
}

/// Log a send its queue refused, as the router logged it before the engine.
/// Relays and searches were dropped without a word, and still are.
fn refused(kind: Kind, error: &TrySendError<()>) {
    match kind {
        Kind::Forward => warn!(%error, "Router dropped message: output channel full"),
        Kind::Global => warn!(%error, "Router dropped broadcast: output channel full"),
        Kind::Reject(_) => warn!(%error, "Router dropped reject message: output channel full"),
        Kind::Answer(NetworkMessageType::I_AM_ROUTER_TO_NETWORK) => {
            warn!(%error, "Router dropped I-Am-Router response: output channel full")
        }
        Kind::Answer(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK) => {
            warn!(%error, "Router dropped Init-Routing-Table-ACK: output channel full")
        }
        Kind::Answer(_) => warn!(%error, "Router dropped Network-Number-Is: output channel full"),
        Kind::Announce(NetworkMessageType::NETWORK_NUMBER_IS) => {
            warn!(%error, "Router dropped Network-Number-Is announcement: output channel full")
        }
        Kind::Announce(_) => {
            warn!(%error, "Router dropped I-Am-Router announcement: output channel full")
        }
        Kind::Relay(_) | Kind::Solicit => {}
        // A kind a later engine adds.
        _ => warn!(%error, "Router dropped a message: output channel full"),
    }
}
