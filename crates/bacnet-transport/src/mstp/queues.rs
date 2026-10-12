//! The transmit queues the core sends from, shared with the transport's
//! send calls, and the clock that maps Tokio time onto the core's.
//!
//! The core lives in the transport's task, but `send_unicast` and
//! `send_broadcast` run on the caller's task and report a full queue or an
//! oversized NPDU at once, as they always have. So the queues sit behind a
//! [`std::sync::Mutex`], which the core locks through [`QueueStorage`] for
//! each queue operation and a send locks for one enqueue. Neither side holds
//! it across an await.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_mstp_core::{
    AllocQueues, DequeuedNpdu, Duration as CoreDuration, Instant as CoreInstant, NetworkPriority,
    QueueCounters, QueueItemInfo, QueueKind, QueueRejectReason, QueueStorage,
};

/// Room in each queue, unicast and broadcast: the host transport's limit
/// before the core, which had one queue of this depth for both.
pub(super) const QUEUE_CAPACITY: usize = 256;

/// The core's queues, and whether the task that sends from them still runs.
#[derive(Clone)]
pub(super) struct SharedQueues(Arc<Mutex<Shared>>);

struct Shared {
    queues: AllocQueues,
    /// Cleared when the transport's task ends, so a send after that fails
    /// instead of queueing a frame nothing will send.
    open: bool,
}

/// Why a send could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnqueueError {
    /// The transport's task has ended.
    Closed,
    /// The queue refused it.
    Rejected(QueueRejectReason),
}

impl SharedQueues {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(Shared {
            queues: AllocQueues::new(QUEUE_CAPACITY, QUEUE_CAPACITY),
            open: true,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Refuse every later send.
    pub(super) fn close(&self) {
        self.lock().open = false;
    }

    /// Queue an NPDU from a send call, at `now` on the core's clock and
    /// expiring `max_age` later, as the core's own enqueue does.
    pub(super) fn enqueue(
        &self,
        destination: u8,
        npdu: &[u8],
        now: CoreInstant,
        max_age: CoreDuration,
    ) -> Result<(), EnqueueError> {
        self.enqueue_until(destination, npdu, now, now.saturating_add(max_age))
    }

    /// Queue an NPDU at `now`, expiring at `expires`.
    pub(super) fn enqueue_until(
        &self,
        destination: u8,
        npdu: &[u8],
        now: CoreInstant,
        expires: CoreInstant,
    ) -> Result<(), EnqueueError> {
        let mut shared = self.lock();
        if !shared.open {
            return Err(EnqueueError::Closed);
        }
        shared
            .queues
            .enqueue_npdu(
                destination,
                npdu,
                npdu_priority(npdu),
                now.as_micros(),
                Some(expires.as_micros()),
            )
            .map_err(EnqueueError::Rejected)
    }

    /// The unicast and broadcast queue counters.
    pub(super) fn stats(&self) -> (QueueCounters, QueueCounters) {
        let shared = self.lock();
        (
            shared.queues.stats(QueueKind::Unicast),
            shared.queues.stats(QueueKind::Broadcast),
        )
    }
}

/// The network priority in an NPDU's control octet (Clause 6.2.2), so the
/// core's queues serve life-safety traffic first; normal for an NPDU too
/// short to carry one.
pub(super) fn npdu_priority(npdu: &[u8]) -> NetworkPriority {
    npdu.get(1)
        .and_then(|control| NetworkPriority::from_u8(control & 0x03))
        .unwrap_or(NetworkPriority::Normal)
}

impl QueueStorage for SharedQueues {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.lock().queues.set_starvation_limit(limit);
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: u64,
        expires_at: Option<u64>,
    ) -> Result<(), QueueRejectReason> {
        self.lock()
            .queues
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: u64) -> usize {
        self.lock().queues.expire(now)
    }

    fn peek(&self, now: u64) -> Option<QueueItemInfo> {
        self.lock().queues.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: u64) -> Option<QueueItemInfo> {
        self.lock().queues.peek_kind(kind, now)
    }

    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        self.lock().queues.dequeue_kind_into(kind, now, scratch)
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.lock().queues.depth(kind)
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        self.lock().queues.stats(kind)
    }
}

/// Tokio time mapped onto the core's microsecond clock from a fixed origin,
/// the moment the transport started. Both the task and the send calls use
/// it, so queue times and core events share one clock.
#[derive(Debug, Clone, Copy)]
pub(super) struct CoreClock {
    origin: tokio::time::Instant,
}

impl CoreClock {
    pub(super) fn new() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }

    /// The core's time now.
    pub(super) fn now(&self) -> CoreInstant {
        self.at(tokio::time::Instant::now())
    }

    /// The core's time at `instant`, rounded down to the microsecond.
    pub(super) fn at(&self, instant: tokio::time::Instant) -> CoreInstant {
        let elapsed = instant.saturating_duration_since(self.origin).as_micros();
        CoreInstant::from_micros(u64::try_from(elapsed).unwrap_or(u64::MAX))
    }

    /// The Tokio instant of a core time. Rounding [`Self::at`] down means a
    /// timer set here never wakes before the core's deadline is due.
    pub(super) fn instant(&self, at: CoreInstant) -> tokio::time::Instant {
        let offset = std::time::Duration::from_micros(at.as_micros());
        self.origin
            .checked_add(offset)
            .unwrap_or_else(|| self.origin + std::time::Duration::from_secs(86_400 * 365))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_comes_from_the_npdu_control_octet() {
        assert_eq!(npdu_priority(&[1, 0x03]), NetworkPriority::LifeSafety);
        assert_eq!(
            npdu_priority(&[1, 0x06]),
            NetworkPriority::CriticalEquipment
        );
        assert_eq!(npdu_priority(&[1, 0x01]), NetworkPriority::Urgent);
        assert_eq!(npdu_priority(&[1, 0x04]), NetworkPriority::Normal);
        assert_eq!(npdu_priority(&[1]), NetworkPriority::Normal);
    }

    #[test]
    fn a_closed_queue_refuses_sends() {
        let queues = SharedQueues::new();
        let now = CoreInstant::ZERO;
        let age = CoreDuration::from_secs(1);
        assert_eq!(queues.enqueue(5, &[1, 0], now, age), Ok(()));
        queues.close();
        assert_eq!(
            queues.enqueue(5, &[1, 0], now, age),
            Err(EnqueueError::Closed)
        );
        assert_eq!(queues.stats().0.enqueued, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_clock_round_trips_without_waking_early() {
        let clock = CoreClock::new();
        tokio::time::advance(std::time::Duration::from_nanos(2_500)).await;
        let now = clock.now();
        assert_eq!(now, CoreInstant::from_micros(2));
        assert!(clock.instant(now) <= tokio::time::Instant::now());
        let deadline = CoreInstant::from_micros(10);
        assert_eq!(clock.at(clock.instant(deadline)), deadline);
    }
}
