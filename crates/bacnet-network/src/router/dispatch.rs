//! Inbound dispatch: what the router does with each frame a port receives.
//!
//! Every port runs one [`PortDispatch`] task. For each frame it takes the
//! engine's lock, hands the frame to the engine (`bacnet-router-core`,
//! #1679), drains the sends the engine asks for onto the port queues, and
//! lets the lock go. The engine decides everything Clause 6 asks of a
//! router, in the order the router always followed: the address bounds,
//! the broadcast rules, network messages (routed first when they name
//! another network), local delivery, routing, rejects and the search for
//! unknown networks. Then, outside the lock, the frame's APDU goes to the
//! local application queue when the engine delivers it here, and a reject
//! addressed to the router goes to its own network-control consumer.

use std::sync::Arc;

use bacnet_router_core::npdu::{AddressField, Malformed};
use bacnet_router_core::router::{DropReason, Frame, Received};
use bacnet_router_core::PortId;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use tokio::sync::mpsc;
use tracing::warn;

use super::actions::drain;
use super::control_policy::ControlGate;
use super::engine::Engine;
use super::hosted_host::{HostedHost, PortGroups};
use super::local_control::LocalControl;
use super::local_delivery::Delivery;
use super::SendRequest;
use crate::layer::{AdmissionSender, ReceivedApdu};

/// What the router, every port's dispatch and the sweep share.
pub(super) struct Shared {
    /// The engine and its clock.
    pub(super) engine: Engine,
    /// The RB-09 wire-control gate.
    pub(super) gate: Arc<ControlGate>,
    /// Each port's group destinations, by port index.
    pub(super) groups: Vec<PortGroups>,
    /// Each port's network number, by port index.
    pub(super) networks: Vec<u16>,
}

impl Shared {
    /// Queue the startup announcements and send them on `send_txs`.
    pub(super) fn announce(&self, send_txs: &[mpsc::Sender<SendRequest>]) {
        let mut router = self.engine.lock();
        router.start();
        drain(&mut router, &[], send_txs);
    }

    /// Run the engine's sweep if it is due, and return when the next is.
    pub(super) fn sweep(&self) -> Option<bacnet_router_core::Instant> {
        let mut host = HostedHost::new(&self.groups, &self.gate, TransportProvenance::unverified());
        let mut router = self.engine.lock();
        router.on_deadline(self.engine.now(), &mut host);
        // The sweep queues no sends; were one to come, it would be refused
        // here rather than left waiting.
        drain(&mut router, &[], &[]);
        router.next_deadline()
    }
}

/// Where the dispatch tasks put what they don't route: the port queues and
/// the local consumers. Only the dispatch tasks hold it, so when
/// [`BACnetRouter::stop`](super::BACnetRouter::stop) ends them, the local
/// receivers see their queues close.
pub(super) struct Outputs {
    /// Each port's send queue, by port index.
    pub(super) send_txs: Vec<mpsc::Sender<SendRequest>>,
    /// The local application receive queue.
    pub(super) local_tx: AdmissionSender<ReceivedApdu>,
    /// The router's own network-control consumer.
    pub(super) local_control: LocalControl,
}

/// What the engine made of a frame, owned, so the frame can move on.
enum Outcome {
    /// Its APDU is for the router's application.
    Local(Delivery),
    /// A reject addressed to the router.
    ToRouter,
    /// Dropped, and counted, for this reason.
    Dropped(DropReason),
    /// Routed, answered or taken: the sends say the rest.
    Handled,
}

/// One port's dispatch task.
pub(super) struct PortDispatch {
    /// What the router shares.
    pub(super) shared: Arc<Shared>,
    /// Where the frames go.
    pub(super) outputs: Arc<Outputs>,
    /// This port's index.
    pub(super) port: u8,
}

impl PortDispatch {
    /// Dispatch every frame the port receives until its channel closes.
    pub(super) async fn run(self, mut rx: mpsc::Receiver<ReceivedNpdu>) {
        while let Some(received) = rx.recv().await {
            self.dispatch(received);
        }
    }

    /// Dispatch one frame. Nothing here waits: the engine's lock is held
    /// for the call and its drain only, and every queue takes `try_send`.
    pub(super) fn dispatch(&self, received: ReceivedNpdu) {
        let (shared, outputs) = (&*self.shared, &*self.outputs);
        let port = PortId(self.port);
        let frame = Frame {
            port,
            source: &received.source_mac,
            link_group: received.link_layer_group,
            npdu: &received.npdu,
        };
        let mut host = HostedHost::new(&shared.groups, &shared.gate, received.provenance);
        let outcome = {
            let mut router = shared.engine.lock();
            let failures = router.counters().write_failures;
            let outcome = match router.receive(frame, shared.engine.now(), &mut host) {
                Received::Local(local) => Outcome::Local(Delivery::of(&local)),
                Received::ToRouter(_) => Outcome::ToRouter,
                Received::Dropped(reason) => Outcome::Dropped(reason),
                Received::Handled => Outcome::Handled,
            };
            // A rewrite or relay that couldn't be written; one the frame's
            // own result names is logged below.
            if router.counters().write_failures != failures
                && !matches!(outcome, Outcome::Dropped(DropReason::Unwritable))
            {
                warn!(port = self.port, "Failed to encode forwarded NPDU");
            }
            drain(&mut router, &received.data_attributes, &outputs.send_txs);
            outcome
        };
        let network = shared
            .networks
            .get(usize::from(self.port))
            .copied()
            .unwrap_or_default();
        match outcome {
            Outcome::Local(delivery) => {
                let _ = outputs
                    .local_tx
                    .try_send_apdu(delivery.application(received, network));
            }
            Outcome::ToRouter => outputs.local_control.deliver(&received),
            Outcome::Dropped(reason) => log_drop(reason, self.port, received.source_mac.len()),
            Outcome::Handled => {}
        }
    }
}

/// Log a dropped frame as the router logged it before the engine. Hop
/// counts are logged by the engine's event; denials, refused network
/// messages and unroutable rejects drop without a word, as they did.
fn log_drop(reason: DropReason, port: u8, source_len: usize) {
    match reason {
        DropReason::SourceTooLong { .. } => warn!(
            octets = source_len,
            limit = bacnet_router_core::MAX_MAC_LEN,
            "Dropping a frame whose link-layer source MAC is over the limit"
        ),
        DropReason::AddressTooLong { field, length } => {
            let octet = match field {
                AddressField::Destination => "DLEN",
                AddressField::Source => "SLEN",
            };
            warn!(
                error = %format_args!(
                    "NPDU {octet}={length} exceeds the {}-octet address limit",
                    bacnet_router_core::MAX_MAC_LEN
                ),
                port,
                "Router refused an NPDU"
            );
        }
        DropReason::Malformed(fault) => {
            let error = match fault {
                Malformed::Truncated => "NPDU truncated",
                Malformed::Version => "unsupported BACnet protocol version",
                Malformed::ReservedNetwork => "NPDU names a reserved network number",
                Malformed::EmptySource => "NPDU source SLEN=0 is invalid",
            };
            warn!(error, port, "Router decode failed");
        }
        DropReason::GlobalWithDadr => {
            warn!("Dropping a global broadcast NPDU that also names a DADR")
        }
        DropReason::BroadcastNotUnconfirmed => {
            warn!("Dropping a broadcast NPDU whose APDU isn't an Unconfirmed-Request")
        }
        DropReason::GroupDadr => warn!(
            port,
            "Dropping a routed NPDU for a group DADR its port's broadcast doesn't stand for, \
             or that isn't an Unconfirmed-Request"
        ),
        DropReason::Unwritable => warn!(port, "Failed to encode forwarded NPDU"),
        DropReason::UnknownPort
        | DropReason::HopCount
        | DropReason::Denied
        | DropReason::BadMessage
        | DropReason::Unroutable => {}
    }
}
