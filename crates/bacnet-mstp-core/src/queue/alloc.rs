//! Allocation-backed queue storage.

use alloc::vec::Vec;

use bacnet_time::Instant;

use crate::counters::QueueCounters;

use super::slots::{expired, Lane, Selected, Slot};
use super::{
    DequeuedNpdu, NetworkPriority, QueueItemInfo, QueueKind, QueueRejectReason, QueueStorage,
    MAX_STANDARD_MPDU_DATA,
};

/// Allocation-backed queues. Capacity is reserved by the constructor, so
/// enqueue and dequeue do not grow a vector after initialization.
///
/// `N` is the longest NPDU a slot holds, as for `HeaplessQueues` (the
/// `heapless` feature): 501 octets by default, and
/// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA) for
/// a core with extended frames on. `N` can be at most 65,535, or the
/// constructors fail to compile.
///
/// Each item is written into a free slot and stays there until it leaves,
/// so no removal moves another item. A slot is first used only when every
/// one before it is in use, so the reserved memory is touched as the queue
/// fills.
#[derive(Debug)]
pub struct AllocQueues<const N: usize = MAX_STANDARD_MPDU_DATA> {
    unicast: Lane<Vec<Slot<N>>>,
    broadcast: Lane<Vec<Slot<N>>>,
    unicast_capacity: usize,
    broadcast_capacity: usize,
    sequence: u64,
    starvation_limit: u8,
}

impl<const N: usize> AllocQueues<N> {
    /// Construct independently bounded queues.
    pub fn new(unicast_capacity: usize, broadcast_capacity: usize) -> Self {
        Self::with_starvation_limit(unicast_capacity, broadcast_capacity, 16)
    }

    /// Construct queues with an explicit service-opportunity starvation limit.
    pub fn with_starvation_limit(
        unicast_capacity: usize,
        broadcast_capacity: usize,
        starvation_limit: u8,
    ) -> Self {
        const {
            assert!(
                N <= Slot::<N>::LONGEST,
                "a queue slot holds at most 65,535 octets"
            );
        };
        Self {
            unicast: Lane::new(
                Vec::with_capacity(unicast_capacity),
                QueueCounters::default(),
            ),
            broadcast: Lane::new(
                Vec::with_capacity(broadcast_capacity),
                QueueCounters::default(),
            ),
            unicast_capacity,
            broadcast_capacity,
            sequence: 0,
            starvation_limit,
        }
    }

    fn select(&self, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit;
        let unicast = self.unicast.select(QueueKind::Unicast, now, limit, None);
        self.broadcast
            .select(QueueKind::Broadcast, now, limit, unicast)
    }

    fn select_kind(&self, kind: QueueKind, now: Instant) -> Option<Selected> {
        self.lane(kind)
            .select(kind, now, self.starvation_limit, None)
    }

    fn lane(&self, kind: QueueKind) -> &Lane<Vec<Slot<N>>> {
        match kind {
            QueueKind::Unicast => &self.unicast,
            QueueKind::Broadcast => &self.broadcast,
        }
    }

    fn lane_mut(&mut self, kind: QueueKind) -> &mut Lane<Vec<Slot<N>>> {
        match kind {
            QueueKind::Unicast => &mut self.unicast,
            QueueKind::Broadcast => &mut self.broadcast,
        }
    }
}

impl<const N: usize> Default for AllocQueues<N> {
    fn default() -> Self {
        Self::new(256, 256)
    }
}

impl<const N: usize> QueueStorage for AllocQueues<N> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.starvation_limit = limit;
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
        let capacity = match kind {
            QueueKind::Unicast => self.unicast_capacity,
            QueueKind::Broadcast => self.broadcast_capacity,
        };
        let sequence = self.sequence;
        let lane = self.lane_mut(kind);
        if payload.len() > N {
            lane.counters.record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        if expired(expires_at, now) {
            lane.counters.record_drop_expired();
            return Err(QueueRejectReason::Expired);
        }
        if lane.depth >= capacity {
            lane.counters.record_drop_full();
            return Err(QueueRejectReason::Full);
        }
        // Fewer items than the capacity: a slot is free, or every slot so
        // far holds one and a new one fits the reservation.
        let index = match lane.slots.iter().position(Slot::is_free) {
            Some(index) => index,
            None => {
                lane.slots.push(Slot::EMPTY);
                lane.slots.len() - 1
            }
        };
        lane.slots[index].fill(destination, payload, priority, now, expires_at, sequence);
        lane.record_enqueue();
        self.sequence = self.sequence.wrapping_add(1);
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
        self.lane_mut(kind).remove(index);
        if scratch.len() < info.len {
            self.lane_mut(kind).counters.record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        self.unicast.bump_waits();
        self.broadcast.bump_waits();
        scratch[..info.len].copy_from_slice(self.lane(kind).slots[index].payload());
        Ok(Some(DequeuedNpdu {
            destination: info.destination,
            kind,
            priority: info.priority,
            len: info.len,
            sequence: info.sequence,
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.lane(kind).depth
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        self.lane(kind).counters
    }
}
