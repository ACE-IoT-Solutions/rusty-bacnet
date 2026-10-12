//! What the B/IP receive path drops, counted by reason (#1624), and the
//! throttle on the one drop it still warns about.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use bacnet_bip_codec::BvllError;
use tokio::time::Instant;
use tracing::{debug, warn};

/// Datagrams a B/IP transport's receive path dropped since the transport was
/// created, by reason. The totals survive stop and restart.
///
/// A dropped datagram counts once, under the first check it fails, in the
/// order the fields are listed. Two drops are counted elsewhere: a datagram
/// whose UDP source is a group address, in
/// [`group_source_drops`](super::BipTransport::group_source_drops), and a
/// Forwarded-NPDU whose origin is one, in
/// [`forwarded_group_origin_drops`](super::BipTransport::forwarded_group_origin_drops).
/// Each field is read on its own, so a snapshot taken while datagrams arrive
/// need not add up across fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiveCounters {
    /// Failed socket reads. Most drop one datagram and reading goes on: one
    /// larger than the receive buffer, one without usable address metadata,
    /// and on Windows the receive errors it reports for an earlier send. Any
    /// other error closes the socket: a broadcast listener closes alone, and
    /// on the socket every send leaves from, the transport stops receiving.
    pub receive_errors: u64,
    /// Datagrams shorter than the 4-octet BVLL header.
    pub too_short: u64,
    /// Datagrams whose BVLC type octet is not B/IP's (0x81), such as other
    /// traffic sent to the port.
    pub not_bip: u64,
    /// Datagrams whose BVLC Length field differs from the datagram's length:
    /// cut short, or with octets after the message.
    pub length_mismatch: u64,
    /// Frames whose BVLC function doesn't fit the address they were sent to:
    /// an Original-Broadcast-NPDU not sent to a broadcast address, or any
    /// function but it and Forwarded-NPDU not sent to this node's own
    /// address (Annex J.2, J.4).
    pub destination_mismatch: u64,
    /// Datagrams a broadcast listener read but doesn't keep, in per-address
    /// mode ([`set_share_port_by_address`](super::BipTransport::set_share_port_by_address)):
    /// a unicast for an address no socket on the port is bound to, or a
    /// broadcast that came in on another interface. Routine on a shared port
    /// or a multihomed host.
    pub listener_mismatch: u64,
    /// BBMD management requests (Read-BDT, Read-FDT, Register-Foreign-Device
    /// and Delete-FDT-Entry) discarded unanswered over the management rate
    /// limit.
    pub rate_limited: u64,
    /// Frames whose BVLC function Annex J doesn't define.
    pub unknown_function: u64,
    /// Frames whose payload doesn't fit their BVLC function's layout. A
    /// malformed BBMD request still draws its NAK, and a malformed
    /// management response still reaches a request waiting for it, which
    /// [`bvlc_client_snapshot`](super::BipTransport::bvlc_client_snapshot)
    /// counts too.
    pub malformed_payload: u64,
    /// Frames from this node's own B/IP address, and Forwarded-NPDUs naming
    /// it as their origin: mostly its own broadcasts coming back, which is
    /// routine.
    pub own_echo: u64,
    /// Forwarded-NPDUs a BBMD refused because their sender is not in its
    /// Broadcast Distribution Table.
    pub forwarded_not_from_peer: u64,
    /// NPDUs dropped because the channel to the network layer was full (or,
    /// once the network layer stopped reading, closed). A BBMD still
    /// forwards a broadcast it couldn't hand up.
    pub npdu_channel_full: u64,
}

/// Why the receive path dropped a datagram: one per [`ReceiveCounters`]
/// field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropReason {
    /// [`ReceiveCounters::receive_errors`].
    ReceiveError,
    /// [`ReceiveCounters::too_short`].
    TooShort,
    /// [`ReceiveCounters::not_bip`].
    NotBip,
    /// [`ReceiveCounters::length_mismatch`].
    LengthMismatch,
    /// [`ReceiveCounters::destination_mismatch`].
    DestinationMismatch,
    /// [`ReceiveCounters::listener_mismatch`].
    ListenerMismatch,
    /// [`ReceiveCounters::rate_limited`].
    RateLimited,
    /// [`ReceiveCounters::unknown_function`].
    UnknownFunction,
    /// [`ReceiveCounters::malformed_payload`].
    MalformedPayload,
    /// [`ReceiveCounters::own_echo`].
    OwnEcho,
    /// [`ReceiveCounters::forwarded_not_from_peer`].
    ForwardedNotFromPeer,
    /// [`ReceiveCounters::npdu_channel_full`].
    NpduChannelFull,
}

/// One slot per [`DropReason`].
const DROP_REASONS: usize = DropReason::NpduChannelFull as usize + 1;

impl DropReason {
    /// Why a datagram's header failed [`BvllFrame::decode`](bacnet_bip_codec::BvllFrame::decode).
    pub(super) fn header(error: &BvllError) -> Self {
        match error {
            BvllError::TooShort { .. } => Self::TooShort,
            BvllError::NotBip { .. } => Self::NotBip,
            // The third and last way the header check fails.
            _ => Self::LengthMismatch,
        }
    }

    /// Why a frame's payload failed [`BvllFrame::message`](bacnet_bip_codec::BvllFrame::message).
    pub(super) fn payload(error: &BvllError) -> Self {
        match error {
            BvllError::UnknownFunction { .. } => Self::UnknownFunction,
            _ => Self::MalformedPayload,
        }
    }
}

/// Least time between two warnings about NPDUs dropped for a full channel.
pub(super) const CHANNEL_FULL_WARNING_INTERVAL: Duration = Duration::from_secs(10);

/// The receive path's drop counts, which the transport keeps across
/// restarts and lends to each receive loop, and the channel-full warning's
/// throttle.
#[derive(Debug, Default)]
pub(super) struct ReceiveDrops {
    counts: [AtomicU64; DROP_REASONS],
    channel_full: Mutex<ChannelFullWarning>,
}

/// When the channel-full warning last went out, and what it hasn't told.
#[derive(Debug, Default)]
struct ChannelFullWarning {
    last: Option<Instant>,
    unreported: u64,
}

impl ReceiveDrops {
    /// Count one datagram dropped for `reason`.
    pub(super) fn count(&self, reason: DropReason) {
        self.counts[reason as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// Count an NPDU dropped because the network layer's channel is full,
    /// and warn at most once per [`CHANNEL_FULL_WARNING_INTERVAL`] with the
    /// number dropped since the last warning, so that a sustained overload
    /// shows without flooding the log; the rest log at debug. `frame` names
    /// the kind of frame dropped. Returns the number reported, when it warns.
    pub(super) fn npdu_channel_full(&self, frame: &'static str) -> Option<u64> {
        self.count(DropReason::NpduChannelFull);
        let now = Instant::now();
        let dropped = {
            let mut warning = self
                .channel_full
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            warning.unreported += 1;
            let quiet = warning
                .last
                .is_some_and(|last| now.duration_since(last) < CHANNEL_FULL_WARNING_INTERVAL);
            if quiet {
                None
            } else {
                warning.last = Some(now);
                Some(std::mem::take(&mut warning.unreported))
            }
        };
        match dropped {
            Some(dropped) => warn!(
                dropped,
                frame, "B/IP: NPDU channel full; dropped incoming frames since the last warning"
            ),
            None => debug!(frame, "B/IP: NPDU channel full, dropping an incoming frame"),
        }
        dropped
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> ReceiveCounters {
        let get = |reason: DropReason| self.counts[reason as usize].load(Ordering::Relaxed);
        ReceiveCounters {
            receive_errors: get(DropReason::ReceiveError),
            too_short: get(DropReason::TooShort),
            not_bip: get(DropReason::NotBip),
            length_mismatch: get(DropReason::LengthMismatch),
            destination_mismatch: get(DropReason::DestinationMismatch),
            listener_mismatch: get(DropReason::ListenerMismatch),
            rate_limited: get(DropReason::RateLimited),
            unknown_function: get(DropReason::UnknownFunction),
            malformed_payload: get(DropReason::MalformedPayload),
            own_echo: get(DropReason::OwnEcho),
            forwarded_not_from_peer: get(DropReason::ForwardedNotFromPeer),
            npdu_channel_full: get(DropReason::NpduChannelFull),
        }
    }
}
