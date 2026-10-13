//! Fixed-capacity queue storage, with no allocator.

use bacnet_time::Instant;

use crate::counters::QueueCounters;

use super::slots::{expired, Lane, Selected, Slot};
use super::{
    DequeuedNpdu, NetworkPriority, QueueItemInfo, QueueKind, QueueRejectReason, QueueStorage,
    MAX_STANDARD_MPDU_DATA,
};

const EMPTY_QUEUE_COUNTERS: QueueCounters = QueueCounters {
    depth: 0,
    high_water: 0,
    enqueued: 0,
    dequeued: 0,
    drops_full: 0,
    drops_oversize: 0,
    drops_expired: 0,
    drops_starved: 0,
};

/// The starvation limit of [`HeaplessQueues::new`].
const DEFAULT_STARVATION_LIMIT: u8 = 16;

/// Fixed-capacity, independently bounded unicast and broadcast queues.
///
/// `U` and `B` are the unicast and broadcast capacities, and `N` the longest
/// NPDU a slot holds: a standard frame's 501 octets by default, and
/// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA) for
/// a core with extended frames on. `N` can be at most 65,535, or the
/// constructors fail to compile.
///
/// Each item is written into a free slot and stays there until it leaves,
/// so no removal moves another item.
///
/// [`Self::new`] is a `const fn` whose result is all zero bits, so the
/// queues can live in a zero-initialized static (`.bss`) and be lent to the
/// core as `&mut` (see the crate docs).
#[derive(Debug)]
pub struct HeaplessQueues<const U: usize, const B: usize, const N: usize = MAX_STANDARD_MPDU_DATA> {
    unicast: Lane<[Slot<N>; U]>,
    broadcast: Lane<[Slot<N>; B]>,
    sequence: u64,
    /// `None` for [`DEFAULT_STARVATION_LIMIT`], so that `new` is all zeros.
    starvation_limit: Option<u8>,
}

impl<const U: usize, const B: usize, const N: usize> HeaplessQueues<U, B, N> {
    /// Construct queues with the default starvation limit of 16 time units.
    pub const fn new() -> Self {
        Self::with_limit(None)
    }

    /// Construct queues with an explicit starvation limit.
    pub const fn with_starvation_limit(starvation_limit: u8) -> Self {
        Self::with_limit(Some(starvation_limit))
    }

    const fn with_limit(starvation_limit: Option<u8>) -> Self {
        const {
            assert!(
                N <= Slot::<N>::LONGEST,
                "a queue slot holds at most 65,535 octets"
            );
        };
        Self {
            unicast: Lane::new([Slot::EMPTY; U], EMPTY_QUEUE_COUNTERS),
            broadcast: Lane::new([Slot::EMPTY; B], EMPTY_QUEUE_COUNTERS),
            sequence: 0,
            starvation_limit,
        }
    }

    fn starvation_limit(&self) -> u8 {
        self.starvation_limit.unwrap_or(DEFAULT_STARVATION_LIMIT)
    }

    fn select(&self, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit();
        let unicast = self.unicast.select(QueueKind::Unicast, now, limit, None);
        self.broadcast
            .select(QueueKind::Broadcast, now, limit, unicast)
    }

    fn select_kind(&self, kind: QueueKind, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit();
        match kind {
            QueueKind::Unicast => self.unicast.select(kind, now, limit, None),
            QueueKind::Broadcast => self.broadcast.select(kind, now, limit, None),
        }
    }

    fn slot(&self, kind: QueueKind, index: usize) -> &Slot<N> {
        match kind {
            QueueKind::Unicast => &self.unicast.slots[index],
            QueueKind::Broadcast => &self.broadcast.slots[index],
        }
    }

    fn counters_mut(&mut self, kind: QueueKind) -> &mut QueueCounters {
        match kind {
            QueueKind::Unicast => &mut self.unicast.counters,
            QueueKind::Broadcast => &mut self.broadcast.counters,
        }
    }
}

impl<const U: usize, const B: usize, const N: usize> Default for HeaplessQueues<U, B, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const U: usize, const B: usize, const N: usize> QueueStorage for HeaplessQueues<U, B, N> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.starvation_limit = Some(limit);
    }

    fn max_npdu_length(&self) -> usize {
        N
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRejectReason> {
        let kind = QueueKind::from_destination(destination);
        if payload.len() > N {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        if expired(expires_at, now) {
            self.counters_mut(kind).record_drop_expired();
            return Err(QueueRejectReason::Expired);
        }
        // The number is taken before the room is checked, so a full queue
        // uses one up too, as it always has here.
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let free = match kind {
            QueueKind::Unicast => self.unicast.free_slot(),
            QueueKind::Broadcast => self.broadcast.free_slot(),
        };
        let Some(slot) = free else {
            self.counters_mut(kind).record_drop_full();
            return Err(QueueRejectReason::Full);
        };
        slot.fill(destination, payload, priority, now, expires_at, sequence);
        match kind {
            QueueKind::Unicast => self.unicast.record_enqueue(),
            QueueKind::Broadcast => self.broadcast.record_enqueue(),
        }
        Ok(())
    }

    fn expire(&mut self, now: Instant) -> usize {
        self.unicast.expire(now) + self.broadcast.expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.select(now).map(|selected| selected.2)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.select_kind(kind, now).map(|selected| selected.2)
    }

    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: Instant,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        self.expire(now);
        let Some((_, index, info)) = self.select_kind(kind, now) else {
            return Ok(None);
        };
        match kind {
            QueueKind::Unicast => self.unicast.remove(index),
            QueueKind::Broadcast => self.broadcast.remove(index),
        }
        if scratch.len() < info.len {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        self.unicast.bump_waits();
        self.broadcast.bump_waits();
        scratch[..info.len].copy_from_slice(self.slot(kind, index).payload());
        Ok(Some(DequeuedNpdu {
            destination: info.destination,
            kind,
            priority: info.priority,
            len: info.len,
            sequence: info.sequence,
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        match kind {
            QueueKind::Unicast => self.unicast.depth,
            QueueKind::Broadcast => self.broadcast.depth,
        }
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        match kind {
            QueueKind::Unicast => self.unicast.counters,
            QueueKind::Broadcast => self.broadcast.counters,
        }
    }
}
