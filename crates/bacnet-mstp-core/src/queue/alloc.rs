//! Allocation-backed queue storage.

use alloc::vec::Vec;

use bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA;

use crate::counters::QueueCounters;

use super::{
    better, expired, DequeuedNpdu, Item, NetworkPriority, QueueClass, QueueItemInfo, QueueKind,
    QueueRejectReason, QueueStorage, MAX_NPDU_LEN,
};

/// Allocation-backed queues. Capacity is reserved by the constructor, so
/// enqueue and dequeue do not grow a vector after initialization.
///
/// `N` is the longest NPDU a slot holds, as for
/// [`HeaplessQueues`](super::HeaplessQueues): 501 octets by default, and
/// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA) for
/// a core with extended frames on.
#[derive(Debug)]
pub struct AllocQueues<const N: usize = MAX_STANDARD_MPDU_DATA> {
    unicast: Vec<Item<N>>,
    broadcast: Vec<Item<N>>,
    unicast_capacity: usize,
    broadcast_capacity: usize,
    network_control: Vec<Item<N>>,
    network_control_capacity: usize,
    sequence: u64,
    starvation_limit: u8,
    unicast_counters: QueueCounters,
    broadcast_counters: QueueCounters,
    network_control_counters: QueueCounters,
}

impl<const N: usize> AllocQueues<N> {
    /// Construct independently bounded queues.
    pub fn new(unicast_capacity: usize, broadcast_capacity: usize) -> Self {
        Self::with_control_capacity(unicast_capacity, broadcast_capacity, 2)
    }

    /// Construct queues with an independently reserved network-control ring.
    pub fn with_control_capacity(
        unicast_capacity: usize,
        broadcast_capacity: usize,
        network_control_capacity: usize,
    ) -> Self {
        Self::with_starvation_limit_and_control_capacity(
            unicast_capacity,
            broadcast_capacity,
            16,
            network_control_capacity,
        )
    }

    /// Construct queues with an explicit service-opportunity starvation limit.
    pub fn with_starvation_limit(
        unicast_capacity: usize,
        broadcast_capacity: usize,
        starvation_limit: u8,
    ) -> Self {
        Self::with_starvation_limit_and_control_capacity(
            unicast_capacity,
            broadcast_capacity,
            starvation_limit,
            2,
        )
    }

    /// Construct queues with explicit starvation and control-capacity bounds.
    pub fn with_starvation_limit_and_control_capacity(
        unicast_capacity: usize,
        broadcast_capacity: usize,
        starvation_limit: u8,
        network_control_capacity: usize,
    ) -> Self {
        Self {
            unicast: Vec::with_capacity(unicast_capacity),
            broadcast: Vec::with_capacity(broadcast_capacity),
            network_control: Vec::with_capacity(network_control_capacity),
            unicast_capacity,
            broadcast_capacity,
            network_control_capacity,
            sequence: 0,
            starvation_limit,
            unicast_counters: QueueCounters::default(),
            broadcast_counters: QueueCounters::default(),
            network_control_counters: QueueCounters::default(),
        }
    }

    fn select(&self, now: u64) -> Option<(QueueKind, usize, QueueItemInfo)> {
        let mut selected: Option<(QueueKind, usize, QueueItemInfo)> = None;
        for (index, item) in self.unicast.iter().copied().enumerate() {
            let info = item.info(QueueKind::Unicast, QueueClass::Data);
            if !expired(info.expires_at, now)
                && better(info, selected.map(|v| v.2), self.starvation_limit)
            {
                selected = Some((QueueKind::Unicast, index, info));
            }
        }
        for (index, item) in self.broadcast.iter().copied().enumerate() {
            let info = item.info(QueueKind::Broadcast, QueueClass::Data);
            if !expired(info.expires_at, now)
                && better(info, selected.map(|v| v.2), self.starvation_limit)
            {
                selected = Some((QueueKind::Broadcast, index, info));
            }
        }
        selected
    }

    fn select_kind(&self, kind: QueueKind, now: u64) -> Option<(usize, QueueItemInfo)> {
        let mut selected: Option<(usize, QueueItemInfo)> = None;
        match kind {
            QueueKind::Unicast => {
                for (index, item) in self.unicast.iter().copied().enumerate() {
                    let info = item.info(kind, QueueClass::Data);
                    if !expired(info.expires_at, now)
                        && better(info, selected.map(|v| v.1), self.starvation_limit)
                    {
                        selected = Some((index, info));
                    }
                }
            }
            QueueKind::Broadcast => {
                for (index, item) in self.broadcast.iter().copied().enumerate() {
                    let info = item.info(kind, QueueClass::Data);
                    if !expired(info.expires_at, now)
                        && better(info, selected.map(|v| v.1), self.starvation_limit)
                    {
                        selected = Some((index, info));
                    }
                }
            }
        }
        selected
    }

    fn bump_waits(&mut self) {
        for item in &mut self.unicast {
            item.service_wait = item.service_wait.saturating_add(1);
        }
        for item in &mut self.broadcast {
            item.service_wait = item.service_wait.saturating_add(1);
        }
        for item in &mut self.network_control {
            item.service_wait = item.service_wait.saturating_add(1);
        }
    }

    fn select_control(&self, now: u64) -> Option<(usize, QueueItemInfo)> {
        let mut selected: Option<(usize, QueueItemInfo)> = None;
        for (index, item) in self.network_control.iter().copied().enumerate() {
            let info = item.info(
                QueueKind::from_destination(item.destination),
                QueueClass::NetworkControl,
            );
            if !expired(info.expires_at, now)
                && better(info, selected.map(|value| value.1), self.starvation_limit)
            {
                selected = Some((index, info));
            }
        }
        selected
    }

    fn counters_mut(&mut self, kind: QueueKind) -> &mut QueueCounters {
        match kind {
            QueueKind::Unicast => &mut self.unicast_counters,
            QueueKind::Broadcast => &mut self.broadcast_counters,
        }
    }

    fn class_counters_mut(&mut self, class: QueueClass, kind: QueueKind) -> &mut QueueCounters {
        match class {
            QueueClass::Data => self.counters_mut(kind),
            QueueClass::NetworkControl => &mut self.network_control_counters,
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
        now: u64,
        expires_at: Option<u64>,
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
        let capacity = match kind {
            QueueKind::Unicast => self.unicast_capacity,
            QueueKind::Broadcast => self.broadcast_capacity,
        };
        let length = match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        };
        if length >= capacity {
            self.counters_mut(kind).record_drop_full();
            return Err(QueueRejectReason::Full);
        }
        let mut owned = Item {
            destination,
            payload,
            priority,
            now,
            expires_at,
            sequence: self.sequence,
            service_wait: 0,
            len: payload.len(),
            payload: [0; N],
        };
        owned.payload[..payload.len()].copy_from_slice(payload);
        self.sequence = self.sequence.wrapping_add(1);
        match kind {
            QueueKind::Unicast => self.unicast.push(owned),
            QueueKind::Broadcast => self.broadcast.push(owned),
        }
        let depth = match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        };
        self.counters_mut(kind).record_enqueue(depth);
        Ok(())
    }

    fn expire(&mut self, now: u64) -> usize {
        let mut removed = 0;
        let mut index = 0;
        while index < self.unicast.len() {
            if expired(self.unicast[index].expires_at, now) {
                self.unicast.remove(index);
                self.unicast_counters.record_drop_expired();
                self.unicast_counters.record_dequeue(self.unicast.len());
                removed += 1;
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.broadcast.len() {
            if expired(self.broadcast[index].expires_at, now) {
                self.broadcast.remove(index);
                self.broadcast_counters.record_drop_expired();
                self.broadcast_counters.record_dequeue(self.broadcast.len());
                removed += 1;
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.network_control.len() {
            if expired(self.network_control[index].expires_at, now) {
                self.network_control.remove(index);
                self.network_control_counters.record_drop_expired();
                self.network_control_counters
                    .record_dequeue(self.network_control.len());
                removed += 1;
            } else {
                index += 1;
            }
        }
        removed
    }

    fn peek(&self, now: u64) -> Option<QueueItemInfo> {
        self.select(now).map(|selected| selected.2)
    }

    fn peek_class(&self, class: QueueClass, now: u64) -> Option<QueueItemInfo> {
        match class {
            QueueClass::Data => self.peek(now),
            QueueClass::NetworkControl => self.select_control(now).map(|selected| selected.1),
        }
    }

    fn peek_kind(&self, kind: QueueKind, now: u64) -> Option<QueueItemInfo> {
        self.select_kind(kind, now).map(|selected| selected.1)
    }

    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        self.expire(now);
        let Some((index, info)) = self.select_kind(kind, now) else {
            return Ok(None);
        };
        let item = match kind {
            QueueKind::Unicast => self.unicast.remove(index),
            QueueKind::Broadcast => self.broadcast.remove(index),
        };
        if scratch.len() < info.len {
            let depth = self.depth(kind);
            let counters = self.counters_mut(kind);
            counters.record_dequeue(depth);
            counters.record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        let depth = self.depth(kind);
        self.counters_mut(kind).record_dequeue(depth);
        self.bump_waits();
        scratch[..item.len].copy_from_slice(&item.payload[..item.len]);
        Ok(Some(DequeuedNpdu {
            destination: item.destination,
            kind,
            class: QueueClass::Data,
            priority: item.priority,
            len: item.len,
            sequence: item.sequence,
        }))
    }

    fn dequeue_class_into(
        &mut self,
        class: QueueClass,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        if class == QueueClass::Data {
            return self.dequeue_into(now, scratch);
        }
        self.expire(now);
        let Some((index, info)) = self.select_control(now) else {
            return Ok(None);
        };
        if scratch.len() < info.len {
            return Err(QueueRejectReason::Oversize);
        }
        let item = self.network_control.remove(index);
        self.network_control_counters
            .record_dequeue(self.network_control.len());
        self.bump_waits();
        scratch[..item.len].copy_from_slice(&item.payload[..item.len]);
        Ok(Some(DequeuedNpdu {
            destination: item.destination,
            kind: QueueKind::from_destination(item.destination),
            class,
            priority: item.priority,
            len: item.len,
            sequence: item.sequence,
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        }
    }

    fn depth_class(&self, class: QueueClass) -> usize {
        match class {
            QueueClass::Data => self.unicast.len() + self.broadcast.len(),
            QueueClass::NetworkControl => self.network_control.len(),
        }
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        match kind {
            QueueKind::Unicast => self.unicast_counters,
            QueueKind::Broadcast => self.broadcast_counters,
        }
    }

    fn stats_class(&self, class: QueueClass) -> QueueCounters {
        match class {
            QueueClass::Data => QueueCounters {
                depth: self
                    .unicast_counters
                    .depth
                    .saturating_add(self.broadcast_counters.depth),
                high_water: self
                    .unicast_counters
                    .high_water
                    .saturating_add(self.broadcast_counters.high_water),
                enqueued: self
                    .unicast_counters
                    .enqueued
                    .saturating_add(self.broadcast_counters.enqueued),
                dequeued: self
                    .unicast_counters
                    .dequeued
                    .saturating_add(self.broadcast_counters.dequeued),
                drops_full: self
                    .unicast_counters
                    .drops_full
                    .saturating_add(self.broadcast_counters.drops_full),
                drops_oversize: self
                    .unicast_counters
                    .drops_oversize
                    .saturating_add(self.broadcast_counters.drops_oversize),
                drops_expired: self
                    .unicast_counters
                    .drops_expired
                    .saturating_add(self.broadcast_counters.drops_expired),
                drops_starved: self
                    .unicast_counters
                    .drops_starved
                    .saturating_add(self.broadcast_counters.drops_starved),
            },
            QueueClass::NetworkControl => self.network_control_counters,
        }
    }
}
