//! The hosted router as it was before the `bacnet-router-core` engine
//! (#1679), kept for the cutover's differential test only. It is deleted,
//! with that test, before the cutover merges, as the legacy copies of #1620,
//! #1670 and #1622 were.
//!
//! The files are `dispatch.rs`, `forwarding.rs`, `control_messages.rs`,
//! `reject.rs`, `group_dadr.rs`, `local_control.rs` and `router_table.rs`
//! as they stood at ab013bda, with the discovery tracker, the ingress record,
//! the local-delivery helper, the startup announcements and the aging sweep
//! of `router/mod.rs` below. The edits are mechanical:
//!
//! - `std::time` became `tokio::time`, so paused Tokio time drives every
//!   timer the copy reads, where the original read the wall clock inline;
//! - module paths point at these copies;
//! - every send goes through [`try_send_as`], which records what kind of
//!   send each was, so the differential test can compare that too;
//! - the gate's `authorize`, which takes this copy's [`IngressContext`], is
//!   the function [`authorize`] here, over the gate's decision;
//! - the test modules the originals declared, and the
//!   `BACnetRouter::network_control_ingress_sequence` getter, were left out;
//! - the startup announcements and the aging sweep, inline in
//!   `start_dispatch`, became the functions [`announce`] and [`sweep`];
//! - [`router_table::RouterTable::pending`] and
//!   [`DiscoveryTracker::solicited`] were added, so the test can read state
//!   the originals kept private.

#![allow(
    dead_code,
    reason = "the cutover's differential test, in the next commit, is the only user"
)]

pub(in crate::router) mod control_messages;
pub(in crate::router) mod dispatch;
pub(in crate::router) mod forwarding;
pub(in crate::router) mod group_dadr;
pub(in crate::router) mod local_control;
pub(in crate::router) mod reject;
pub(in crate::router) mod router_table;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::NetworkMessageType;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tracing::{debug, warn};

use super::control_policy::{ControlAuthContext, ControlClass, ControlGate, ControlTrust};
use super::SendRequest;
use crate::layer::ReceivedApdu;
use router_table::RouterTable;

/// The `SendRequest` constructors of `router/mod.rs`.
impl SendRequest {
    pub(in crate::router) fn broadcast(npdu: Bytes) -> Self {
        Self::broadcast_with_attributes(npdu, &[])
    }

    /// Ingress-triggered unicast: carries the ingress data attributes instead
    /// of silently dropping them (RB-03). Locally-originated messages with no
    /// ingress attributes pass an empty slice.
    pub(in crate::router) fn unicast_with_attributes(
        npdu: Bytes,
        mac: MacAddr,
        data_attributes: &[DataAttribute],
    ) -> Self {
        Self::Unicast {
            npdu,
            mac,
            data_attributes: data_attributes.to_vec(),
        }
    }

    /// Ingress-triggered broadcast: carries the ingress data attributes
    /// instead of silently dropping them (RB-03). Locally-originated
    /// messages with no ingress attributes use [`Self::broadcast`].
    pub(in crate::router) fn broadcast_with_attributes(
        npdu: Bytes,
        data_attributes: &[DataAttribute],
    ) -> Self {
        Self::Broadcast {
            npdu,
            data_attributes: data_attributes.to_vec(),
        }
    }
}

/// Why the copy sent an NPDU, recorded at each send site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::router) enum SendKind {
    /// Routed toward its DNET.
    Forward,
    /// A global broadcast passed on.
    Global,
    /// A network message of this type passed on.
    Relay(u8),
    /// A reject this router sends, with this reason.
    Reject(u8),
    /// An answer of this type.
    Answer(u8),
    /// A startup announcement of this type.
    Announce(u8),
    /// A Who-Is-Router for an unknown network.
    Solicit,
}

thread_local! {
    /// What each send that went into a queue carried, and its kind.
    static SENT: RefCell<Vec<(Bytes, SendKind)>> = const { RefCell::new(Vec::new()) };
    /// Sends a full or closed queue refused.
    static REFUSED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `tx.try_send(request)`, recording `kind` when it went in, and counting
/// a refusal when it didn't.
pub(in crate::router) fn try_send_as(
    tx: &mpsc::Sender<SendRequest>,
    request: SendRequest,
    kind: SendKind,
) -> Result<(), TrySendError<SendRequest>> {
    let npdu = match &request {
        SendRequest::Unicast { npdu, .. } | SendRequest::Broadcast { npdu, .. } => npdu.clone(),
    };
    if let Err(error) = tx.try_send(request) {
        REFUSED.with(|refused| refused.set(refused.get() + 1));
        return Err(error);
    }
    SENT.with(|sent| sent.borrow_mut().push((npdu, kind)));
    Ok(())
}

/// The sends refused on this thread since the last call.
pub(in crate::router) fn take_refused() -> u64 {
    REFUSED.with(|refused| refused.replace(0))
}

/// The sends recorded on this thread since the last call.
pub(in crate::router) fn take_sent() -> Vec<(Bytes, SendKind)> {
    SENT.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

/// Bounded unknown-destination discovery (RB-06, Clauses 6.5/6.6.3.1).
///
/// Per-DNET single inflight Who-Is solicitation: simultaneous unknowns coalesce
/// to one broadcast, rate-limited to one per DNET per 5s, entries expire after
/// 30s via the aging reaper, at most 256 pending DNETs, reserved never
/// solicited. The caller still fails with NOT_DIRECTLY_CONNECTED (honest
/// retryable); packets are never buffered and no per-packet retries occur.
/// `cancel` (via `BACnetRouter::stop`) drops all pending discovery.
const DISCOVERY_COOLDOWN: Duration = Duration::from_secs(5);
const DISCOVERY_MAX_AGE: Duration = Duration::from_secs(30);
const DISCOVERY_MAX_PENDING: usize = 256;

#[derive(Debug, Default)]
pub(in crate::router) struct DiscoveryTracker {
    last_solicited: HashMap<u16, Instant>,
    cancelled: bool,
}

impl DiscoveryTracker {
    /// Decide whether an unknown `dnet` should solicit now; records it if so.
    pub(in crate::router) fn should_solicit_at(&mut self, dnet: u16, now: Instant) -> bool {
        if self.cancelled || dnet == 0 || dnet == 0xFFFF {
            return false;
        }
        if let Some(last) = self.last_solicited.get(&dnet) {
            if now.duration_since(*last) < DISCOVERY_COOLDOWN {
                return false;
            }
        } else if self.last_solicited.len() >= DISCOVERY_MAX_PENDING {
            return false;
        }
        self.last_solicited.insert(dnet, now);
        true
    }

    /// Wall-clock wrapper for dispatch paths.
    pub(in crate::router) fn should_solicit(&mut self, dnet: u16) -> bool {
        self.should_solicit_at(dnet, Instant::now())
    }

    /// Reap entries older than 30s; returns the reaped DNETs ascending.
    pub(in crate::router) fn expire_at(&mut self, now: Instant) -> Vec<u16> {
        let mut out: Vec<u16> = self
            .last_solicited
            .iter()
            .filter(|(_, last)| now.duration_since(**last) > DISCOVERY_MAX_AGE)
            .map(|(net, _)| *net)
            .collect();
        out.sort_unstable();
        for net in &out {
            self.last_solicited.remove(net);
        }
        out
    }

    /// Cancel all pending discovery (router stop).
    pub(in crate::router) fn cancel(&mut self) {
        self.cancelled = true;
        self.last_solicited.clear();
    }

    pub(in crate::router) fn len(&self) -> usize {
        self.last_solicited.len()
    }

    pub(in crate::router) fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Added for the differential test: each network looked for, with when.
    pub(in crate::router) fn solicited(&self) -> Vec<(u16, Instant)> {
        let mut out: Vec<(u16, Instant)> = self
            .last_solicited
            .iter()
            .map(|(net, at)| (*net, *at))
            .collect();
        out.sort_unstable();
        out
    }
}

/// Immutable ingress facts carried to the network-message admission point
/// ([`dispatch::dispatch_network_message`] / `handle_network_message`).
///
/// This is a plain record of what arrived, free of trust assertions: ingress port
/// identity, the immediate link peer (`source_mac`), link-layer group flag,
/// data-link attributes, and the decoded NPDU envelope (source/destination,
/// hop count, message type, vendor ID). It asserts no provenance or trust —
/// a claimed SNET/SADR is carried, never verified here.
///
/// RB-07 threads [`TransportProvenance`] here; RB-09 `control_policy`
/// consumes it at the same admission point for protected-control decisions.
#[derive(Debug, Clone)]
pub(crate) struct IngressContext {
    /// Dispatch index of the ingress port.
    pub port_idx: usize,
    /// BACnet network number assigned to the ingress port.
    pub port_network: u16,
    /// Immediate link peer that sent the frame (SA), not the routed origin.
    pub source_mac: MacAddr,
    /// Whether the frame arrived via a data-link multicast/broadcast.
    pub link_layer_group: bool,
    /// Data-link attributes supplied with the frame, if any.
    pub data_attributes: Vec<DataAttribute>,
    /// Honest transport + origin provenance, immutable by value (RB-07).
    /// Threaded, never decided on here.
    pub provenance: TransportProvenance,
    /// Decoded NPDU, including envelope (source/destination, hop count,
    /// message type, vendor ID) and control payload.
    pub npdu: Npdu,
}

impl IngressContext {
    /// In-memory test ingress: link-broadcast delivery with no attributes.
    pub(crate) fn test_local(
        port_idx: usize,
        port_network: u16,
        source_mac: &[u8],
        npdu: Npdu,
    ) -> Self {
        Self {
            port_idx,
            port_network,
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            npdu,
        }
    }

    /// In-memory test ingress with explicit provenance (RB-07).
    pub(crate) fn test_local_with_provenance(
        port_idx: usize,
        port_network: u16,
        source_mac: &[u8],
        npdu: Npdu,
        provenance: TransportProvenance,
    ) -> Self {
        Self {
            port_idx,
            port_network,
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            provenance,
            npdu,
        }
    }
}

/// `ControlGate::authorize` of `control_policy.rs`: build redacted context
/// from immutable ingress + validated targets and decide. Call after
/// validation, before any table lock. `targets_omitted` marks Busy/Available
/// omitted-list scope (all served via peer).
pub(in crate::router) fn authorize(
    control: &ControlGate,
    ingress: &IngressContext,
    class: ControlClass,
    targets: &[u16],
    targets_omitted: bool,
) -> bool {
    let ctx = ControlAuthContext {
        port_idx: ingress.port_idx,
        port_network: ingress.port_network,
        source_mac: ingress.source_mac.clone(),
        provenance: ingress.provenance,
        trust: ControlTrust::from_provenance(ingress.provenance),
        message_type: ingress.npdu.message_type.unwrap_or(0xFF),
        vendor_id: ingress.npdu.vendor_id,
        source_net: ingress.npdu.source.as_ref().map(|s| s.network),
        dest_net: ingress.npdu.destination.as_ref().map(|d| d.network),
        target_nets: targets.to_vec(),
        targets_omitted,
    };
    control.decide(&ctx, class)
}

/// Transfer original-ingress metadata to every local router delivery branch
/// (`router/local_delivery.rs`).
pub(in crate::router) mod local_delivery {
    use super::*;

    pub(in crate::router) fn application(
        received: ReceivedNpdu,
        npdu: Npdu,
        port_network: u16,
        is_group: bool,
    ) -> ReceivedApdu {
        let global_broadcast = crate::layer::is_global_broadcast(npdu.destination.as_ref());
        ReceivedApdu {
            apdu: npdu.payload,
            source_mac: received.source_mac,
            ingress_network: Some(port_network),
            source_network: npdu.source,
            link_layer_group: received.link_layer_group,
            is_group,
            global_broadcast,
            data_attributes: received.data_attributes,
            provenance: received.provenance,
            direct_response: received.direct_response,
            reply_tx: received.reply_tx,
        }
    }
}

/// The startup announcements of `start_dispatch`: an I-Am-Router on each
/// port listing the networks reachable via the other ports.
pub(in crate::router) fn announce(send_txs: &[mpsc::Sender<SendRequest>], port_networks: &[u16]) {
    // Announce I-Am-Router-To-Network on each port listing networks reachable via other ports.
    for (port_idx, tx) in send_txs.iter().enumerate() {
        let other_networks: Vec<u16> = port_networks
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != port_idx)
            .map(|(_, net)| *net)
            .collect();

        if other_networks.is_empty() {
            continue;
        }

        let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
        for net in &other_networks {
            payload.put_u16(*net);
        }

        let payload_len = payload.len();
        let response = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: payload.freeze(),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + payload_len);
        if let Err(e) = encode_npdu(&mut buf, &response) {
            warn!("Failed to encode I-Am-Router NPDU: {e}");
            continue;
        }

        if let Err(e) = try_send_as(
            tx,
            SendRequest::broadcast(buf.freeze()),
            SendKind::Announce(0x01),
        ) {
            warn!(%e, "Router dropped I-Am-Router announcement: output channel full");
        }
    }
}

/// One pass of the aging task of `start_dispatch`: purge stale learned
/// routes, hardened pending slots and discovery entries. `last_used` never
/// extends `last_seen` expiry.
pub(super) async fn sweep(
    aging_table: &Arc<Mutex<RouterTable>>,
    aging_discovery: &Arc<Mutex<DiscoveryTracker>>,
) {
    let max_age = Duration::from_secs(300); // 5 minutes
    let now = Instant::now();
    let mut tbl = aging_table.lock().await;
    let purged = tbl.purge_stale_at(now, max_age);
    tbl.clear_expired_busy();
    let expired_pending = tbl.expire_pending_at(now);
    drop(tbl);
    let mut disc = aging_discovery.lock().await;
    let expired_discovery = disc.expire_at(now);
    drop(disc);
    for net in purged {
        debug!(network = net, "Purged stale route");
    }
    for net in expired_pending {
        debug!(network = net, "Expired hardened pending challenger");
    }
    for net in expired_discovery {
        debug!(network = net, "Expired discovery solicitation");
    }
}
