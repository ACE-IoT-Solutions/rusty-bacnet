//! The watch hub (#2067): one managed BACnet subscription or poller per
//! point, however many listeners share it.
//!
//! [`Hub::watch`] hands out a [`Lease`] on a [`Point`]. The first lease on a
//! point starts its entry: a task that reads the point once, then asks the
//! device for a finite COV subscription (SubscribeCOV for an object,
//! SubscribeCOVProperty for one property) through the client's managed
//! subscriptions, which renew it before it lapses. A device that refuses
//! COV is polled instead, never faster than [`MIN_POLL`]. Later leases on
//! the point share the entry; the last one dropped stops it, and the task
//! cancels its subscription the way the TUI's watch list does when a row
//! goes. A listener joining while the last one leaves gets either the
//! running entry or a new one with its own subscriber process identifier:
//! both happen under one lock, so no entry is shared after its stop.
//!
//! The hub keeps each point's latest values ([`Snapshot`]), so reading a
//! watched point sends nothing. COV notifications come from one receiver of
//! the client's channel ([`BACnetClient::cov_notifications`]), the path the
//! TUI's watch list and the managed renewals already use; the hub adds no
//! second notification path. The receiver never blocks the client: a burst
//! it falls behind on is counted as lagged, and changes to one point within
//! [`WatchOptions::coalesce`] are folded into one signal and counted as
//! coalesced, while the values themselves are always the latest.
//!
//! Every request an entry sends (its first read, the subscription, polls)
//! is charged to the hub's own [`Account`], whose token bucket
//! ([`WatchOptions::bucket`]) holds the hub to its own rate besides the
//! engine's limits. A shared point costs the same whatever the number of
//! listeners, and notifications cost nothing. Renewals take the device's
//! gate but no rate slot, as in the TUI; cancels skip the hub's bucket so a
//! shutdown isn't held to it.
//!
//! Lifetimes are finite by construction ([`Lifetime`] can't be zero): a
//! subscription that never lapses is a write in [`crate::class`], and the
//! hub never sends one.
//!
//! [`BACnetClient::cov_notifications`]: bacnet_client::client::BACnetClient::cov_notifications

mod dispatch;
mod entry;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_client::client::{CovTarget, ReceivedCOVNotification};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::class::{self, Class};
use crate::engine::Engine;
use crate::format::RawRow;
use crate::limits::{Account, TokenBucket};

/// The properties an object watch reads: what an agent or an operator needs
/// to reason about a point (value, status, units, priorities, increment).
pub const POINT_PROPERTIES: [PropertyIdentifier; 10] = [
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::RELIABILITY,
    PropertyIdentifier::OUT_OF_SERVICE,
    PropertyIdentifier::UNITS,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::COV_INCREMENT,
];

/// The shortest time between two polls of a point.
pub const MIN_POLL: Duration = Duration::from_secs(10);

/// Subscriber process identifiers start here, clear of the small numbers
/// the TUI's watch rows use.
const PROCESS_BASE: u32 = 0x4D43_0000;

/// A COV subscription's lifetime in seconds: never zero, so never the
/// subscription that doesn't lapse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifetime(NonZeroU32);

impl Lifetime {
    /// Five minutes, the TUI's default.
    pub const DEFAULT: Self = Self(NonZeroU32::new(300).expect("300 is not zero"));

    /// A lifetime of `secs`; `None` for 0.
    pub fn new(secs: u32) -> Option<Self> {
        let secs = NonZeroU32::new(secs)?;
        debug_assert_eq!(class::of_subscribe(secs.get()), Class::Read);
        Some(Self(secs))
    }

    /// The lifetime in seconds, at least 1.
    pub fn secs(self) -> u32 {
        self.0.get()
    }

    fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.secs()))
    }
}

/// The hub's settings.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Each subscription's lifetime.
    pub lifetime: Lifetime,
    /// How long before a lifetime ends the renewal goes. It covers a wait
    /// for the device's gate and the renewal's own request with retries.
    pub renewal_margin: Duration,
    /// Time between two polls of a point the device won't subscribe; at
    /// least [`MIN_POLL`]. A device that stops answering is polled less
    /// often, up to a minute apart.
    pub poll_every: Duration,
    /// After a subscription fails for want of an answer (rather than a
    /// refusal), how long the point is polled before subscribing again.
    pub retry_cov: Duration,
    /// Changes to one point within this window make one signal.
    pub coalesce: Duration,
    /// Points watched at once, those still cancelling included.
    pub max_points: usize,
    /// The hub's own rate, taken by each request an entry sends; `None`
    /// for none beyond the engine's limits.
    pub bucket: Option<Arc<TokenBucket>>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            lifetime: Lifetime::DEFAULT,
            renewal_margin: Duration::from_secs(60),
            poll_every: MIN_POLL,
            retry_cov: Duration::from_secs(300),
            coalesce: Duration::from_secs(1),
            max_points: 256,
            bucket: None,
        }
    }
}

impl WatchOptions {
    /// Check the settings: polls no faster than [`MIN_POLL`], a renewal
    /// margin inside the lifetime, room for one point.
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_every < MIN_POLL {
            return Err(format!("polls go at most every {} s", MIN_POLL.as_secs()));
        }
        if self.renewal_margin.is_zero() || self.renewal_margin >= self.lifetime.duration() {
            return Err("the renewal margin must be inside the lifetime".into());
        }
        if self.max_points == 0 {
            return Err("max_points must be at least 1".into());
        }
        if self.coalesce > Duration::from_secs(60) {
            return Err("coalesce must be a minute or less".into());
        }
        Ok(())
    }
}

/// What a watch follows on an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Watched {
    /// The object: SubscribeCOV, its [`POINT_PROPERTIES`] read and polled.
    Object,
    /// One property, or one element of it: SubscribeCOVProperty.
    Property {
        /// The property.
        property: PropertyIdentifier,
        /// The array element, if one.
        index: Option<u32>,
    },
}

/// A watched point: an object of a device, whole or one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    /// The device instance, looked up in the client's discovery table.
    pub device: u32,
    /// The object.
    pub object: ObjectIdentifier,
    /// The object or one property.
    pub watched: Watched,
}

/// Where a point's values come from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// COV notifications under a managed subscription.
    Cov,
    /// Polls, because the device refused COV or stopped renewing it.
    Poll,
}

/// How a point's watch is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Its first read hasn't finished.
    Starting,
    /// Subscribed, or the last poll answered.
    Live,
    /// The device answered the last read with an error for the whole
    /// object (see [`Snapshot::note`]).
    Failed,
    /// The device didn't answer the last request.
    Unreachable,
    /// The watch ended (the hub shut down).
    Stopped,
}

/// A point's latest values and how its watch is going.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Each property's latest raw value, or why the device gave none.
    pub rows: Vec<RawRow>,
    /// Where the values come from; `None` before the first read.
    pub source: Option<Source>,
    /// How the watch is going.
    pub state: State,
    /// Why the point is polled or not live, as text.
    pub note: Option<String>,
    /// The hub's sequence number at the last change; 0 before any.
    pub seq: u64,
    /// When the values last changed.
    pub updated: Option<Instant>,
    /// COV notifications applied.
    pub notifications: u64,
    /// Changes folded into another's signal.
    pub coalesced: u64,
    /// Polls answered.
    pub polls: u64,
    /// Renewals the device took.
    pub renewals: u64,
}

impl Snapshot {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            source: None,
            state: State::Starting,
            note: None,
            seq: 0,
            updated: None,
            notifications: 0,
            coalesced: 0,
            polls: 0,
            renewals: 0,
        }
    }
}

/// Why a watch wasn't started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// The hub watches its most points already.
    Full {
        /// [`WatchOptions::max_points`].
        max: usize,
    },
    /// The hub has shut down.
    Shut,
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full { max } => write!(f, "the hub watches its most points ({max})"),
            Self::Shut => f.write_str("the hub has shut down"),
        }
    }
}

impl std::error::Error for WatchError {}

/// The hub's footprint, for a session view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubStats {
    /// Points watched.
    pub points: usize,
    /// Entries still running, those cancelling included.
    pub tasks: usize,
    /// Points under a COV subscription.
    pub cov: usize,
    /// Points polled.
    pub polled: usize,
    /// [`WatchOptions::max_points`].
    pub max_points: usize,
    /// Requests the entries sent.
    pub requests: u32,
    /// How long they waited for the limits.
    pub throttled: Duration,
    /// COV notifications lost because the hub's receiver fell behind.
    pub lagged: u64,
}

/// One entry's state, shared by its task, its leases and the dispatcher.
struct Shared {
    point: Point,
    process: u32,
    state: Mutex<EntryState>,
    /// Carries the hub's counter at each change it signals.
    signal: watch::Sender<u64>,
    /// Set once to stop the entry.
    stop: watch::Sender<bool>,
}

struct EntryState {
    snapshot: Snapshot,
    /// Where the device is, once looked up: notifications must come from
    /// there.
    target: Option<CovTarget>,
    last_signal: Option<Instant>,
    /// A change waits for the end of the coalescing window.
    pending: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, EntryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(&self) -> Snapshot {
        self.lock().snapshot.clone()
    }

    fn stop(&self) {
        self.stop.send_replace(true);
    }

    async fn stopped(&self) {
        let mut stop = self.stop.subscribe();
        // The sender lives as long as `self`, so the wait only ends on a stop.
        let _ = stop.wait_for(|stopped| *stopped).await;
    }

    /// Record a change made under `state` and signal it at once, unless one
    /// went within `window`: then the change waits, and the instant it is
    /// due is returned the first time.
    fn changed(
        &self,
        state: &mut EntryState,
        seq: &AtomicU64,
        window: Duration,
    ) -> Option<Instant> {
        let now = Instant::now();
        state.snapshot.seq = seq.fetch_add(1, Ordering::Relaxed) + 1;
        match state.last_signal {
            Some(at) if now.saturating_duration_since(at) < window => {
                if state.pending {
                    state.snapshot.coalesced += 1;
                    None
                } else {
                    state.pending = true;
                    Some(at + window)
                }
            }
            _ => {
                if state.pending {
                    state.snapshot.coalesced += 1;
                }
                state.pending = false;
                state.last_signal = Some(now);
                self.signal.send_replace(state.snapshot.seq);
                None
            }
        }
    }

    /// Signal a change that waited for its window.
    fn flush(&self) {
        let mut state = self.lock();
        if state.pending {
            state.pending = false;
            state.last_signal = Some(Instant::now());
            self.signal.send_replace(state.snapshot.seq);
        }
    }

    /// Change the watch's state and note without a value change; signalled
    /// at once, since the state matters to listeners.
    fn set_state(
        &self,
        seq: &AtomicU64,
        source: Option<Source>,
        state: State,
        note: Option<String>,
    ) {
        let mut entry = self.lock();
        let snapshot = &mut entry.snapshot;
        let new_source = source.is_some() && snapshot.source != source;
        if !new_source && snapshot.state == state && snapshot.note == note {
            return;
        }
        if source.is_some() {
            snapshot.source = source;
        }
        snapshot.state = state;
        snapshot.note = note;
        self.changed(&mut entry, seq, Duration::ZERO);
    }
}

/// The entries, by point and by subscriber process identifier.
struct Table {
    entries: HashMap<Point, Slot>,
    by_process: HashMap<u32, Arc<Shared>>,
    next_process: u32,
    shut: bool,
}

struct Slot {
    shared: Arc<Shared>,
    leases: usize,
}

/// What leases and the dispatcher share without the engine.
struct Registry {
    table: Mutex<Table>,
    seq: AtomicU64,
    lagged: AtomicU64,
    coalesce: Duration,
    /// Entry tasks running, those cancelling included.
    tasks: watch::Sender<usize>,
}

impl Registry {
    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One lease on `shared` ends: the last one stops the entry.
    fn release(&self, shared: &Arc<Shared>) {
        let mut table = self.table();
        let Some(slot) = table.entries.get_mut(&shared.point) else {
            return;
        };
        if !Arc::ptr_eq(&slot.shared, shared) {
            return;
        }
        slot.leases -= 1;
        if slot.leases == 0 {
            table.entries.remove(&shared.point);
            table.by_process.remove(&shared.process);
            drop(table);
            shared.stop();
        }
    }

    /// Apply a COV notification to the entry it names; the instant a
    /// held-back signal is due, the first time one is held back.
    fn deliver(&self, received: &ReceivedCOVNotification) -> Option<(Instant, Arc<Shared>)> {
        let process = received.notification.subscriber_process_identifier;
        let shared = Arc::clone(self.table().by_process.get(&process)?);
        let mut state = shared.lock();
        if !dispatch::belongs(&shared.point, state.target.as_ref(), received) {
            return None;
        }
        dispatch::merge(&mut state.snapshot.rows, &shared.point, received);
        state.snapshot.notifications += 1;
        state.snapshot.updated = Some(Instant::now());
        let due = shared.changed(&mut state, &self.seq, self.coalesce);
        drop(state);
        due.map(|at| (at, shared))
    }
}

/// A lease on a watched point: while any lease on it lives, the hub keeps
/// it watched. Dropping the last one stops the watch and cancels the
/// subscription.
pub struct Lease {
    shared: Arc<Shared>,
    registry: Arc<Registry>,
    signal: watch::Receiver<u64>,
}

impl Lease {
    /// The point.
    pub fn point(&self) -> Point {
        self.shared.point
    }

    /// Its latest values and state.
    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot()
    }

    /// Wait for the next signalled change. Changes within the coalescing
    /// window make one signal, and a listener that falls behind sees only
    /// the latest.
    pub async fn changed(&mut self) {
        // The sender lives as long as the entry, which this lease holds.
        if self.signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.registry.release(&self.shared);
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("point", &self.shared.point)
            .finish()
    }
}

struct Inner<T: TransportPort + 'static> {
    engine: Arc<Engine<T>>,
    options: WatchOptions,
    registry: Arc<Registry>,
    account: Arc<Account>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    stop_dispatch: watch::Sender<bool>,
}

/// The watch hub over one engine. Cheap to clone: clones share one hub.
pub struct Hub<T: TransportPort + 'static> {
    inner: Arc<Inner<T>>,
}

impl<T: TransportPort + 'static> Clone for Hub<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TransportPort + 'static> Hub<T> {
    /// A hub over `engine`, under checked `options`. It starts its
    /// notification receiver, so it must be made inside a Tokio runtime.
    pub fn new(engine: Arc<Engine<T>>, options: WatchOptions) -> Result<Self, String> {
        options.validate()?;
        let registry = Arc::new(Registry {
            table: Mutex::new(Table {
                entries: HashMap::new(),
                by_process: HashMap::new(),
                next_process: PROCESS_BASE,
                shut: false,
            }),
            seq: AtomicU64::new(0),
            lagged: AtomicU64::new(0),
            coalesce: options.coalesce,
            tasks: watch::Sender::new(0),
        });
        let (stop_dispatch, stopped) = watch::channel(false);
        let notifications = engine.client().cov_notifications();
        let dispatcher = tokio::spawn(dispatch::run(Arc::clone(&registry), notifications, stopped));
        let account = Arc::new(Account::new(options.bucket.clone()));
        Ok(Self {
            inner: Arc::new(Inner {
                engine,
                options,
                registry,
                account,
                handles: Mutex::new(vec![dispatcher]),
                stop_dispatch,
            }),
        })
    }

    /// The hub's settings.
    pub fn options(&self) -> &WatchOptions {
        &self.inner.options
    }

    /// A lease on `point`, starting its watch if it has none.
    pub fn watch(&self, point: Point) -> Result<Lease, WatchError> {
        let registry = &self.inner.registry;
        let (shared, fresh) = {
            let mut table = registry.table();
            if table.shut {
                return Err(WatchError::Shut);
            }
            if let Some(slot) = table.entries.get_mut(&point) {
                slot.leases += 1;
                (Arc::clone(&slot.shared), false)
            } else {
                let max = self.inner.options.max_points;
                if *registry.tasks.borrow() >= max {
                    return Err(WatchError::Full { max });
                }
                let mut process = table.next_process;
                // 0 is skipped when the count wraps.
                while process == 0 || table.by_process.contains_key(&process) {
                    process = process.wrapping_add(1);
                }
                table.next_process = process.wrapping_add(1);
                let shared = Arc::new(Shared {
                    point,
                    process,
                    state: Mutex::new(EntryState {
                        snapshot: Snapshot::new(),
                        target: None,
                        last_signal: None,
                        pending: false,
                    }),
                    signal: watch::Sender::new(0),
                    stop: watch::Sender::new(false),
                });
                table.entries.insert(
                    point,
                    Slot {
                        shared: Arc::clone(&shared),
                        leases: 1,
                    },
                );
                table.by_process.insert(process, Arc::clone(&shared));
                registry.tasks.send_modify(|tasks| *tasks += 1);
                (shared, true)
            }
        };
        if fresh {
            let task = tokio::spawn(entry::run(Arc::clone(&self.inner), Arc::clone(&shared)));
            let mut handles = self
                .inner
                .handles
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            handles.retain(|handle| !handle.is_finished());
            handles.push(task);
        }
        let signal = shared.signal.subscribe();
        Ok(Lease {
            shared,
            registry: Arc::clone(registry),
            signal,
        })
    }

    /// `point`'s latest values, if it is watched: nothing is sent.
    pub fn peek(&self, point: &Point) -> Option<Snapshot> {
        let shared = Arc::clone(&self.inner.registry.table().entries.get(point)?.shared);
        Some(shared.snapshot())
    }

    /// The hub's sequence number: every change takes the next one, so a
    /// point changed since an earlier reading of this when its
    /// [`Snapshot::seq`] is greater.
    pub fn seq(&self) -> u64 {
        self.inner.registry.seq.load(Ordering::Relaxed)
    }

    /// The hub's footprint now.
    pub fn stats(&self) -> HubStats {
        let registry = &self.inner.registry;
        let entries: Vec<Arc<Shared>> = registry
            .table()
            .entries
            .values()
            .map(|slot| Arc::clone(&slot.shared))
            .collect();
        let sources: Vec<Option<Source>> = entries
            .iter()
            .map(|shared| shared.lock().snapshot.source)
            .collect();
        HubStats {
            points: entries.len(),
            tasks: *registry.tasks.borrow(),
            cov: sources.iter().filter(|s| **s == Some(Source::Cov)).count(),
            polled: sources.iter().filter(|s| **s == Some(Source::Poll)).count(),
            max_points: self.inner.options.max_points,
            requests: self.inner.account.requests(),
            throttled: self.inner.account.throttled(),
            lagged: registry.lagged.load(Ordering::Relaxed),
        }
    }

    /// Stop every watch, wait up to `wait` for their cancels, and stop the
    /// notification receiver. Later [`watch`](Self::watch) calls fail.
    /// True when every cancel finished in time; the subscriptions of those
    /// that didn't lapse with their lifetime.
    pub async fn shutdown(&self, wait: Duration) -> bool {
        let entries: Vec<Arc<Shared>> = {
            let mut table = self.inner.registry.table();
            table.shut = true;
            table.by_process.clear();
            table.entries.drain().map(|(_, slot)| slot.shared).collect()
        };
        for shared in &entries {
            shared.stop();
        }
        let mut tasks = self.inner.registry.tasks.subscribe();
        let done = tokio::time::timeout(wait, tasks.wait_for(|running| *running == 0))
            .await
            .is_ok();
        self.inner.stop_dispatch.send_replace(true);
        let handles: Vec<JoinHandle<()>> = self
            .inner
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for handle in handles {
            // A cancel still waiting past the deadline is given up: its
            // subscription lapses with its lifetime.
            if !done {
                handle.abort();
            }
            let _ = handle.await;
        }
        if !done {
            tracing::warn!(
                "gave up waiting for COV cancels; those subscriptions end with their lifetime"
            );
        }
        done
    }
}

#[cfg(test)]
mod tests;
