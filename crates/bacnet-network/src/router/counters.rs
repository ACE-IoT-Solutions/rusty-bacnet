//! The hosted router's counts: the totals since start of what the engine
//! counts.
//!
//! The engine counts as the portable cores do: `u32` counts it hands over,
//! and starts again, each time they are taken
//! ([`Router::take_counters`](bacnet_router_core::Router::take_counters)).
//! The hosted router takes them each time it lets the engine's lock go and
//! adds them to these totals, so its users read cumulative `u64` counts.

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
}
