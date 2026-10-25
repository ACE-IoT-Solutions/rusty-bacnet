//! The session's request limits.
//!
//! [`RequestLimits`] is shared by every worker that asks devices anything:
//! a global cap on how many requests a second leave across every device,
//! and one gate per device, so a device never has more than one of our
//! confirmed requests outstanding, whether the browser, the watch list, a
//! write or a managed COV renewal sent it (#1791). Small devices often
//! handle one transaction at a time.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// How long, in seconds, the request-rate meter ([`RequestLimits::recent`])
/// remembers a request.
pub const METER_WINDOW_SECS: u64 = 10;

/// Spaces requests evenly: at most `per_second` a second across every job,
/// the browser's and the watch list's alike. Each caller takes the next free
/// slot and sleeps until it; the lock is held only to take the slot, never
/// across the sleep. It also remembers when recent requests went, for the
/// request-rate meter.
///
/// It also holds each device's gate: a one-permit semaphore that a request
/// to the device holds from before its slot until its answer (see
/// [`turn`](Self::turn)). The std mutex around the map is held only to look
/// a gate up, never across an await.
pub struct RequestLimits {
    interval: Duration,
    state: Mutex<Slots>,
    gates: Mutex<HashMap<u32, Arc<Semaphore>>>,
}

/// A request's turn at its device: the device's gate, held until this is
/// dropped, on every path out of the request (an answer, an error, or the
/// request's future dropped).
#[must_use = "the device's gate is held only while the turn lives"]
pub struct Turn {
    _permit: Option<OwnedSemaphorePermit>,
}

#[derive(Default)]
struct Slots {
    /// The next free slot.
    next: Option<Instant>,
    /// The slots handed out within the meter's window, oldest first.
    recent: VecDeque<Instant>,
}

/// How long the meter remembers a request.
const METER_WINDOW: Duration = Duration::from_secs(METER_WINDOW_SECS);

impl RequestLimits {
    /// A cap of `per_second` requests a second; 0 means no cap.
    pub fn new(per_second: u32) -> Self {
        let interval = match per_second {
            0 => Duration::ZERO,
            n => Duration::from_secs(1) / n,
        };
        Self {
            interval,
            state: Mutex::new(Slots::default()),
            gates: Mutex::new(HashMap::new()),
        }
    }

    /// `device`'s gate: one permit, held by whichever request to it is
    /// outstanding. A managed COV subscription takes it for its renewals.
    pub fn gate(&self, device: u32) -> Arc<Semaphore> {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            gates
                .entry(device)
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    /// Wait for `device`'s gate, then for this request's slot. The turn
    /// holds the gate until it is dropped, so keep it for exactly the one
    /// request: a job that takes a second turn while holding the first
    /// waits on itself.
    ///
    /// The gate comes first so a request queued behind another one to the
    /// same device holds no slot meanwhile. The wait is cancel-safe: a
    /// dropped wait leaves the semaphore's queue.
    pub async fn turn(&self, device: u32) -> Turn {
        // The gate is never closed, so the permit always comes.
        let permit = self.gate(device).acquire_owned().await.ok();
        self.acquire().await;
        Turn { _permit: permit }
    }

    /// Wait until `device`'s gate is free and a request could go at once,
    /// taking neither; see [`wait_free`](Self::wait_free) for why.
    pub async fn wait_device_free(&self, device: u32) {
        drop(self.gate(device).acquire_owned().await);
        self.wait_free().await;
    }

    /// The instant this request may go, reserving it.
    fn reserve(&self, now: Instant) -> Instant {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = state.next.map_or(now, |next| next.max(now));
        state.next = Some(slot + self.interval);
        while state
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= METER_WINDOW)
        {
            state.recent.pop_front();
        }
        state.recent.push_back(slot);
        slot
    }

    /// Wait for this request's slot under the rate cap alone; a request to
    /// a device takes a [`turn`](Self::turn), which includes one.
    pub async fn acquire(&self) {
        let slot = self.reserve(Instant::now());
        if !self.interval.is_zero() {
            tokio::time::sleep_until(slot).await;
        }
    }

    /// Wait until a request could go at once, without reserving a slot: a
    /// caller that decides only then what to send (the name reads) spends
    /// no slot on work withdrawn while it waited.
    ///
    /// That holds only while the caller doesn't await between this and its
    /// [`acquire`](Self::acquire): deciding what to send must be
    /// synchronous (the TUI's name reads take their batch under a std
    /// mutex). An await there lets other jobs book the free slot first, and
    /// the caller's request then waits for a later one, for rows that may
    /// have been withdrawn meanwhile.
    pub async fn wait_free(&self) {
        loop {
            let next = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next;
            match next {
                Some(at) if at > Instant::now() => tokio::time::sleep_until(at).await,
                _ => return,
            }
        }
    }

    /// Requests whose slot fell in the meter's window up to now.
    pub fn recent(&self) -> usize {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .recent
            .iter()
            .filter(|at| **at <= now && now.saturating_duration_since(**at) < METER_WINDOW)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requests are spaced `1 / per_second` apart, whatever the caller,
    /// and a cap of zero never waits.
    #[tokio::test(start_paused = true)]
    async fn the_rate_cap_spaces_requests_evenly() {
        let cap = RequestLimits::new(4);
        let start = Instant::now();
        let mut slots = Vec::new();
        for _ in 0..3 {
            cap.acquire().await;
            slots.push(Instant::now() - start);
        }
        assert_eq!(
            slots,
            [
                Duration::ZERO,
                Duration::from_millis(250),
                Duration::from_millis(500)
            ]
        );
        // After an idle second the next request goes at once.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let before = Instant::now();
        cap.acquire().await;
        assert_eq!(Instant::now(), before);
        let none = RequestLimits::new(0);
        let before = Instant::now();
        for _ in 0..10 {
            none.acquire().await;
        }
        assert_eq!(Instant::now(), before);
    }

    /// Waiting for a free slot reserves none: the next request still goes
    /// as soon as it would have.
    #[tokio::test(start_paused = true)]
    async fn waiting_for_a_free_slot_takes_none() {
        let cap = RequestLimits::new(4);
        let start = Instant::now();
        cap.acquire().await;
        cap.wait_free().await;
        assert_eq!(Instant::now() - start, Duration::from_millis(250));
        cap.wait_free().await;
        cap.acquire().await;
        assert_eq!(Instant::now() - start, Duration::from_millis(250));
        assert_eq!(cap.recent(), 2);
    }
}
