//! The hosted router's counts: the totals since start of what the engine
//! counts.
//!
//! The engine counts as the portable cores do: `u32` counts it hands over,
//! and starts again, each time they are taken
//! ([`Router::take_counters`](bacnet_router_core::Router::take_counters)).
//! The hosted router takes them each time it lets the engine's lock go and
//! adds them to these totals, so its users read cumulative `u64` counts.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What the hosted router's engine has counted since the router started:
/// what it dropped, sent and was refused. Each total saturates at
/// `u64::MAX`. They never change a decision, and name no peer.
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `Counters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counters {
    /// Frames refused because the link source, the DLEN or the SLEN was
    /// past [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) (#1141,
    /// #1198).
    pub address_length_drops: u64,
    /// Frames dropped for a DADR beside DNET 0xFFFF (#1379).
    pub global_broadcast_dadr_drops: u64,
    /// Broadcasts dropped for carrying an APDU other than an
    /// Unconfirmed-Request (#1491).
    pub broadcast_pdu_type_drops: u64,
    /// Frames dropped for a DADR that is a group address on its delivery
    /// port: an APDU other than an Unconfirmed-Request, a network message,
    /// or anything for a group the port refuses (#1504, #1677). An
    /// Unconfirmed-Request sent as the port's broadcast instead isn't
    /// counted.
    pub group_dadr_drops: u64,
    /// Frames that didn't parse as an NPDU for any other reason.
    pub malformed: u64,
    /// Network messages whose data or addressing the router refused.
    pub bad_messages: u64,
    /// Frames that came on a port the router doesn't have, or whose route
    /// names one.
    pub unknown_port: u64,
    /// Relays not sent because the hop count had run out.
    pub hop_count_drops: u64,
    /// Protected network messages the control policy refused.
    pub denied: u64,
    /// Received rejects that were neither for this router nor routable.
    pub unroutable_rejects: u64,
    /// Rejects this router queued.
    pub rejects: u64,
    /// Who-Is-Router searches for unknown networks queued.
    pub solicitations: u64,
    /// Outputs that couldn't be written: longer than the scratch, or with
    /// a field no NPDU may carry, such as an SNET of 0.
    pub write_failures: u64,
    /// Sends left out because their egress port can't carry them: by the
    /// router's own check, or refused by the port's transport as too long,
    /// at once or later.
    pub egress_too_long: u64,
    /// Sends a port reported refused, at once or later.
    pub refused: u64,
    /// Actions dropped because a call came before they were polled.
    pub undrained: u64,
    /// Router-Busy announcements started for a clogged port, repeats
    /// included ([`RouterOptions::congestion`](super::RouterOptions::congestion), #1697).
    pub busy_announced: u64,
    /// Router-Available announcements started for a port that drained
    /// (#1697).
    pub available_announced: u64,
    /// Global broadcasts, and the network messages the router passes on,
    /// dropped as repeats
    /// ([`RouterOptions::duplicates`](super::RouterOptions::duplicates),
    /// #1698, #1810, #2020).
    pub duplicate_drops: u64,
    /// Global broadcasts and Who-Is-Router queries dropped because their
    /// SNET and SADR name the router itself: its own, come back round a
    /// loop (#2021).
    pub own_source_drops: u64,
    /// Networks a received reject or flow-control message named whose
    /// learned route goes through another router, left unchanged
    /// ([`RouterOptions::next_hop_rejects`](super::RouterOptions::next_hop_rejects), #1698).
    pub not_next_hop: u64,
    /// Requests the router's own application sent through the engine
    /// ([`BACnetRouter::originate`](super::BACnetRouter::originate), #1699).
    pub originated: u64,
}

/// Monotonic forwarding counters for one configured router port.
///
/// The position and network number identify the configured port. The counters
/// are cumulative from router start, saturate at `u64::MAX`, and remain
/// readable after the router has stopped. They deliberately complement, and
/// do not replace, the router-wide [`Counters`] snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RouterPortCounters {
    /// Zero-based position in the original router configuration.
    pub config_index: usize,
    /// Configured BACnet network number.
    pub network_number: u16,
    /// Concrete Rust transport type supplied to this router instance. When
    /// ports are wrapped in a transport enum, this names the wrapper rather
    /// than its active variant.
    pub transport_kind: String,
    /// Local transport MAC after startup, formatted as hexadecimal octets.
    pub identity: String,
    /// Data NPDUs successfully sent as unicasts by this port.
    pub forwarded_unicast: u64,
    /// Data NPDU copies successfully sent as broadcasts by this port.
    pub forwarded_broadcast: u64,
    /// Data forwarding attempts that reached a transport but failed to send.
    pub send_errors: u64,
    /// Data forwarding attempts refused because this port's output queue was
    /// full.
    pub output_full_drops: u64,
    /// Ingress data NPDUs for which no usable route existed.
    pub no_route_drops: u64,
    /// Accepted data forwarding attempts cancelled during router shutdown.
    pub shutdown_drops: u64,
}

/// Short alias for callers that refer to the per-port snapshot simply as
/// `PortCounters`.
pub type PortCounters = RouterPortCounters;

/// The atomic state behind one public port snapshot.
#[derive(Default)]
pub(super) struct PortCounterState {
    forwarded_unicast: AtomicU64,
    forwarded_broadcast: AtomicU64,
    send_errors: AtomicU64,
    output_full_drops: AtomicU64,
    no_route_drops: AtomicU64,
    shutdown_drops: AtomicU64,
}

impl PortCounterState {
    fn increment(counter: &AtomicU64) {
        let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
    }

    fn add(counter: &AtomicU64, value: usize) {
        let value = u64::try_from(value).unwrap_or(u64::MAX);
        let _ = counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(value))
        });
    }

    pub(super) fn forwarded_unicast(&self) {
        Self::increment(&self.forwarded_unicast);
    }

    pub(super) fn forwarded_broadcast(&self) {
        Self::increment(&self.forwarded_broadcast);
    }

    pub(super) fn send_error(&self) {
        Self::increment(&self.send_errors);
    }

    pub(super) fn output_full(&self) {
        Self::increment(&self.output_full_drops);
    }

    pub(super) fn no_route(&self) {
        Self::increment(&self.no_route_drops);
    }

    pub(super) fn shutdown(&self) {
        Self::increment(&self.shutdown_drops);
    }

    pub(super) fn shutdown_many(&self, value: usize) {
        Self::add(&self.shutdown_drops, value);
    }
}

/// Stable metadata and shared atomic state for one configured port.
#[derive(Clone)]
pub(super) struct PortCounterEntry {
    config_index: usize,
    network_number: u16,
    transport_kind: String,
    identity: String,
    pub(super) state: Arc<PortCounterState>,
}

impl PortCounterEntry {
    pub(super) fn new(
        config_index: usize,
        network_number: u16,
        transport_kind: String,
        identity: String,
    ) -> Self {
        Self {
            config_index,
            network_number,
            transport_kind,
            identity,
            state: Arc::new(PortCounterState::default()),
        }
    }

    #[cfg(test)]
    pub(super) fn test(network_number: u16, config_index: usize) -> Self {
        Self::new(
            config_index,
            network_number,
            "test".to_string(),
            String::new(),
        )
    }

    pub(super) fn snapshot(&self) -> RouterPortCounters {
        RouterPortCounters {
            config_index: self.config_index,
            network_number: self.network_number,
            transport_kind: self.transport_kind.clone(),
            identity: self.identity.clone(),
            forwarded_unicast: self.state.forwarded_unicast.load(Ordering::Relaxed),
            forwarded_broadcast: self.state.forwarded_broadcast.load(Ordering::Relaxed),
            send_errors: self.state.send_errors.load(Ordering::Relaxed),
            output_full_drops: self.state.output_full_drops.load(Ordering::Relaxed),
            no_route_drops: self.state.no_route_drops.load(Ordering::Relaxed),
            shutdown_drops: self.state.shutdown_drops.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_stable_and_saturating() {
        let entry = PortCounterEntry::new(2, 42, "fixture".into(), "01:02".into());
        entry.state.forwarded_unicast();
        entry.state.forwarded_broadcast();
        entry.state.send_error();
        entry.state.output_full();
        entry.state.no_route();
        entry.state.shutdown();
        let snapshot = entry.snapshot();
        assert_eq!(snapshot.config_index, 2);
        assert_eq!(snapshot.network_number, 42);
        assert_eq!(snapshot.transport_kind, "fixture");
        assert_eq!(snapshot.identity, "01:02");
        assert_eq!(snapshot.forwarded_unicast, 1);
        assert_eq!(snapshot.forwarded_broadcast, 1);
        assert_eq!(snapshot.send_errors, 1);
        assert_eq!(snapshot.output_full_drops, 1);
        assert_eq!(snapshot.no_route_drops, 1);
        assert_eq!(snapshot.shutdown_drops, 1);

        entry
            .state
            .forwarded_unicast
            .store(u64::MAX, Ordering::Relaxed);
        entry.state.shutdown_many(usize::MAX);
        let saturated = entry.snapshot();
        assert_eq!(saturated.forwarded_unicast, u64::MAX);
        assert_eq!(saturated.shutdown_drops, u64::MAX);
    }
}
