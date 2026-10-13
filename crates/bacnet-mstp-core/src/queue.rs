//! Bounded, observable MS/TP NPDU queues.
//!
//! The queue implementations deliberately own their payload bytes.  This is
//! important for the core API: a caller may hand an NPDU to the core from a
//! short-lived receive buffer, while the core must retain it until a token is
//! available.  The alloc implementation reserves all of its item storage in
//! its constructor; enqueue and dequeue do not grow a vector.
//!
//! A dequeue lends the NPDU from where it was queued rather than copying it
//! out, so the core builds a frame straight from the queue's slot, with no
//! NPDU-sized buffer on its stack.

#[cfg(any(feature = "alloc", feature = "heapless"))]
use bacnet_mstp_codec::MAX_STANDARD_MPDU_DATA;
use bacnet_time::Instant;

use crate::counters::QueueCounters;

/// Destination class used by the two independently bounded queues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueKind {
    /// A frame addressed to one MAC address.
    Unicast,
    /// A frame addressed to the MS/TP broadcast MAC (`0xff`).
    Broadcast,
}

impl QueueKind {
    /// The MS/TP broadcast MAC address.
    pub const BROADCAST_DESTINATION: u8 = 0xff;

    /// Classify a destination MAC.
    pub const fn from_destination(destination: u8) -> Self {
        if destination == Self::BROADCAST_DESTINATION {
            Self::Broadcast
        } else {
            Self::Unicast
        }
    }
}

/// BACnet network priorities, in increasing service order.
///
/// Clause 6.2.2 assigns the numeric values shown here.  Consequently
/// [`LifeSafety`](Self::LifeSafety) is selected before
/// [`CriticalEquipment`](Self::CriticalEquipment), then [`Urgent`](Self::Urgent),
/// and finally [`Normal`](Self::Normal).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkPriority {
    /// Normal traffic (`0`).
    Normal = 0,
    /// Urgent traffic (`1`).
    Urgent = 1,
    /// Critical-equipment traffic (`2`).
    CriticalEquipment = 2,
    /// Life-safety traffic (`3`).
    LifeSafety = 3,
}

impl NetworkPriority {
    /// Convert the wire priority value, rejecting reserved values.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Urgent),
            2 => Some(Self::CriticalEquipment),
            3 => Some(Self::LifeSafety),
            _ => None,
        }
    }

    /// Return the Clause 6.2.2 numeric value.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// The reason an NPDU was rejected by a queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueRejectReason {
    /// The destination queue has no free slot.
    Full,
    /// The payload is too long: larger than the queue's owned slot or, from
    /// [`MasterCore::enqueue_npdu`](crate::MasterCore::enqueue_npdu), than
    /// the station sends
    /// ([`MstpCoreConfig::max_npdu_length`](crate::MstpCoreConfig::max_npdu_length)).
    /// A dequeue also returns it when it dropped an item longer than the
    /// caller takes (see [`QueueStorage::dequeue`]).
    Oversize,
    /// The supplied expiry is at or before the enqueue time, or an item
    /// reached its expiry boundary while waiting.
    Expired,
    /// Reserved for policy layers that reject a request which has exceeded a
    /// starvation budget.  Queue selection itself never drops a starved item.
    Starved,
}

/// An NPDU taken off its queue, with its octets lent by the queue.
///
/// The item has left the queue, but [`Self::npdu`] borrows the queue: the
/// octets stay where they were queued until the queue is next used
/// mutably, so the caller reads or sends them from there instead of copying
/// them out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DequeuedNpdu<'a> {
    /// Destination MAC address.
    pub destination: u8,
    /// Whether this was taken from the unicast or broadcast queue.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
    /// The NPDU, lent by the queue.
    pub npdu: &'a [u8],
}

/// A non-mutating view of the next item selected by queue policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueItemInfo {
    /// Destination MAC address.
    pub destination: u8,
    /// Queue class.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Payload length.
    pub len: usize,
    /// When the item was queued, on the core's timeline.
    pub enqueued_at: Instant,
    /// When the item expires, on the same timeline, if it does.
    pub expires_at: Option<Instant>,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
    /// Number of service opportunities this item has waited.
    pub service_wait: u8,
}

/// A bounded storage backend for queued NPDUs.
pub trait QueueStorage {
    /// Apply the core's service-opportunity starvation limit.
    fn set_starvation_limit(&mut self, limit: u8);

    /// The longest NPDU one queue slot holds. A core with
    /// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
    /// on refuses queues whose slots hold less than
    /// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA);
    /// nothing else reads it.
    ///
    /// The default is a standard frame's 501 octets, so storage that doesn't
    /// say is refused for extended frames: it fails closed. Storage whose
    /// slots hold extended NPDUs overrides it.
    fn max_npdu_length(&self) -> usize {
        bacnet_mstp_codec::MAX_STANDARD_MPDU_DATA
    }

    /// Add a payload to the destination queue.
    ///
    /// `expires_at` is an absolute time on the same timeline as `now`, the
    /// core's. Expiry is inclusive: an item is expired when
    /// `now >= expires_at`.
    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRejectReason>;

    /// Remove expired items at `now`, returning the number removed.
    fn expire(&mut self, now: Instant) -> usize;

    /// Inspect the item that policy would service next.
    fn peek(&self, now: Instant) -> Option<QueueItemInfo>;

    /// Inspect the next item in one destination class.
    ///
    /// The core uses this form when applying its configured
    /// broadcast/unicast service ratio.
    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo>;

    /// Remove the item that policy would service next, after removing the
    /// expired ones, and lend its NPDU until the queue is next used mutably.
    ///
    /// An item longer than `max_len` octets is dropped instead, counted in
    /// `drops_oversize`, and [`QueueRejectReason::Oversize`] is returned.
    /// The core passes the longest NPDU its delivery slots hold, its `NPDU`,
    /// so such an item could never be sent; leaving it queued would keep it
    /// at the head.
    fn dequeue(
        &mut self,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRejectReason> {
        let Some(info) = self.peek(now) else {
            return Ok(None);
        };
        self.dequeue_kind(info.kind, now, max_len)
    }

    /// Remove the next item from one destination class and lend its NPDU,
    /// as [`Self::dequeue`] does, with its oversize rule.
    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRejectReason>;

    /// Current depth for one destination class.
    fn depth(&self, kind: QueueKind) -> usize;

    /// Maximum depth and drop counters for one destination class.
    fn stats(&self, kind: QueueKind) -> QueueCounters;

    /// The total number of queued NPDUs.
    fn total_depth(&self) -> usize {
        self.depth(QueueKind::Unicast) + self.depth(QueueKind::Broadcast)
    }
}

/// Queues kept elsewhere, such as in a static, which the core borrows rather
/// than holds.
impl<T: QueueStorage + ?Sized> QueueStorage for &mut T {
    fn set_starvation_limit(&mut self, limit: u8) {
        (**self).set_starvation_limit(limit);
    }

    fn max_npdu_length(&self) -> usize {
        (**self).max_npdu_length()
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRejectReason> {
        (**self).enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: Instant) -> usize {
        (**self).expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        (**self).peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        (**self).peek_kind(kind, now)
    }

    fn dequeue(
        &mut self,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRejectReason> {
        (**self).dequeue(now, max_len)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRejectReason> {
        (**self).dequeue_kind(kind, now, max_len)
    }

    fn depth(&self, kind: QueueKind) -> usize {
        (**self).depth(kind)
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        (**self).stats(kind)
    }

    fn total_depth(&self) -> usize {
        (**self).total_depth()
    }
}

// The slots and selection policy are shared by the two queue backends, so
// they build only when one of them does.
#[cfg(any(feature = "alloc", feature = "heapless"))]
mod slots;

#[cfg(feature = "heapless")]
mod heapless;

#[cfg(feature = "heapless")]
pub use heapless::HeaplessQueues;

#[cfg(feature = "alloc")]
mod alloc;

#[cfg(feature = "alloc")]
pub use alloc::AllocQueues;
#[cfg(all(test, any(feature = "alloc", feature = "heapless")))]
mod tests;
