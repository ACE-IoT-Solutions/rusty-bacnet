//! Subscriptions (design §6): `subscriptions/listen` of the 2026-07-28
//! revision, which `rmcp` 3.5.1 implements. Updates go out as
//! `notifications/resources/updated` carrying only the URI; the client then
//! reads the resource.
//!
//! The SDK asks for the accepted filter synchronously, before it sends the
//! acknowledgment, so the watches are taken there ([`Subs::reserve`]): the
//! acknowledgment lists only the URIs honoured. A point URI is honoured
//! when it parses, its device is in the discovery table and in reach, and
//! both caps leave room: [`Caps::watches`](crate::policy::Caps::watches)
//! points per principal (the same point on two streams counts once) and
//! the watch hub's cap for the process. The leases wait for the stream's
//! `listen` call ([`Subs::claim`]), matched by their URI list: two streams
//! asking for the same URIs hold the same points, so either may take
//! either's leases. A reservation no stream claims within
//! [`UNCLAIMED`] (the acknowledgment couldn't be sent) is dropped.
//!
//! The listen loop waits on a [`Follow`]: the leases' signals, the
//! discovery table's events and an alarm tick. Its arms send notifications
//! and refresh alarms; the futures it leaves pending meanwhile wait on
//! watch and broadcast channels only, never a lock (#1498). Legacy
//! subscriptions ([`crate::legacy`]) follow theirs the same way, and their
//! holds live here too, so every way of watching shares the caps.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_client::client::{DeviceEvent, DeviceEventKind};
use bacnet_ops::engine::Engine;
use bacnet_ops::watch::{Changes, Lease, Point, WatchError};
use bacnet_transport::port::TransportPort;
use futures_util::stream::{FuturesUnordered, StreamExt};
use rmcp::ErrorData;
use rmcp::service::SubscriptionContext;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;
use tokio::time::{Instant, Interval};

use crate::context::Ctx;
use crate::resources::uri::{self, Uri};

/// Why a watch is refused once the session has ended or the hub stopped.
pub(crate) const SHUTTING_DOWN: &str = "shutting-down";

/// How long a reservation waits for its stream.
const UNCLAIMED: Duration = Duration::from_secs(30);

/// Reservations waiting at once; past it the oldest is dropped.
const MAX_PENDING: usize = 16;

/// The fastest the device list is announced as changed.
const DEVICES_EVERY: Duration = Duration::from_secs(1);

/// How long a `bacnet_watch` point stays watched without being asked for:
/// as long as its cursor lasts.
pub(crate) const TOOL_IDLE: Duration = Duration::from_secs(600);

/// How often subscribed alarm resources are polled.
pub(crate) const ALARMS_EVERY: Duration = Duration::from_secs(60);

/// What the principal watches, through listen streams, legacy
/// subscriptions and `bacnet_watch`.
pub(crate) struct Subs {
    state: Mutex<SubsState>,
    /// Bumped whenever the legacy subscriptions change, for the session's
    /// notifier ([`crate::legacy::notify`]).
    legacy_changed: watch::Sender<u64>,
}

impl Default for Subs {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            legacy_changed: watch::Sender::new(0),
        }
    }
}

#[derive(Default)]
struct SubsState {
    /// Each point watched, and how many holders (streams, the tool) hold
    /// it: the per-principal cap counts the points.
    held: HashMap<Point, usize>,
    /// Reservations made for acknowledgments, waiting for their streams.
    pending: Vec<Pending>,
    /// `bacnet_watch`'s points and when each was last asked for.
    tool: HashMap<Point, (Instant, Lease)>,
    /// The session's legacy `resources/subscribe` URIs, each point URI
    /// with its hold.
    legacy: HashMap<String, Option<Hold>>,
    /// Set when the session ends ([`Subs::clear`]): nothing new is
    /// watched after it.
    closed: bool,
}

/// One legacy subscription's watch on a point: counted against the
/// principal's cap, and its lease held, until it is dropped. It points
/// back weakly, so a hold left in a dropped server's state frees itself.
pub(crate) struct Hold {
    subs: Weak<Subs>,
    point: Point,
    lease: Lease,
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(subs) = self.subs.upgrade() {
            subs.release(&[self.point]);
        }
    }
}

struct Pending {
    at: Instant,
    uris: Vec<String>,
    claim: Claim,
}

/// The points one stream holds, counted against the principal's cap
/// until it is dropped, and their leases until the stream takes them.
pub(crate) struct Claim {
    subs: Arc<Subs>,
    points: Vec<Point>,
    leases: Vec<(String, Lease)>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.subs.release(&self.points);
    }
}

impl Subs {
    fn lock(&self) -> MutexGuard<'_, SubsState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Points the principal watches now.
    pub(crate) fn held(&self) -> usize {
        self.lock().held.len()
    }

    fn release(&self, points: &[Point]) {
        let mut state = self.lock();
        for point in points {
            if let Some(count) = state.held.get_mut(point) {
                *count -= 1;
                if *count == 0 {
                    state.held.remove(point);
                }
            }
        }
    }

    /// A lease on `point` for a new holder, if the principal's cap and the
    /// hub's leave room.
    pub(crate) fn take<T: TransportPort + 'static>(
        &self,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<Lease, &'static str> {
        let hub = ctx.hub.as_ref().ok_or("passive")?;
        let mut state = self.lock();
        if state.closed {
            return Err(SHUTTING_DOWN);
        }
        if !state.held.contains_key(&point) && state.held.len() >= ctx.policy.caps.watches {
            return Err("principal-cap");
        }
        let lease = hub.watch(point).map_err(|e| match e {
            WatchError::Shut => SHUTTING_DOWN,
            WatchError::Full { .. } => "process-cap",
        })?;
        *state.held.entry(point).or_default() += 1;
        Ok(lease)
    }

    /// A hold on `point` for a legacy subscription, under both caps.
    pub(crate) fn hold<T: TransportPort + 'static>(
        self: &Arc<Self>,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<Hold, &'static str> {
        let lease = self.take(ctx, point)?;
        Ok(Hold {
            subs: Arc::downgrade(self),
            point,
            lease,
        })
    }

    /// Whether the session subscribes to `uri` (legacy).
    pub(crate) fn legacy_has(&self, uri: &str) -> bool {
        self.lock().legacy.contains_key(uri)
    }

    /// Add the legacy subscription `uri`, with its point's hold, unless the
    /// session already holds `max` URIs. One added meanwhile by another
    /// call stays, and `hold` goes.
    pub(crate) fn legacy_add(
        &self,
        uri: &str,
        hold: Option<Hold>,
        max: usize,
    ) -> Result<(), &'static str> {
        let mut state = self.lock();
        let added = if state.closed {
            Err(SHUTTING_DOWN)
        } else if state.legacy.contains_key(uri) {
            Ok(false)
        } else if state.legacy.len() >= max {
            Err("principal-cap")
        } else {
            state.legacy.insert(uri.to_string(), hold);
            Ok(true)
        };
        drop(state);
        // A hold not stored goes when this returns, after the lock: its
        // release takes it.
        if added? {
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        Ok(())
    }

    /// Drop the legacy subscription `uri`, if the session has it.
    pub(crate) fn legacy_remove(&self, uri: &str) {
        let removed = self.lock().legacy.remove(uri);
        if removed.is_some() {
            // The guard is gone: the hold's release takes the lock.
            drop(removed);
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
    }

    /// The legacy URIs, each point URI with its change signal.
    pub(crate) fn legacy_snapshot(&self) -> Vec<(String, Option<Changes>)> {
        self.lock()
            .legacy
            .iter()
            .map(|(uri, hold)| (uri.clone(), hold.as_ref().map(|h| h.lease.changes())))
            .collect()
    }

    /// Legacy URIs the session holds.
    pub(crate) fn legacy_count(&self) -> usize {
        self.lock().legacy.len()
    }

    /// Wakes when the legacy subscriptions change.
    pub(crate) fn legacy_watch(&self) -> watch::Receiver<u64> {
        self.legacy_changed.subscribe()
    }

    /// Drop `bacnet_watch` points not asked for within `idle`.
    pub(crate) fn expire_tool(&self, idle: Duration) {
        let expired: Vec<(Point, Lease)> = {
            let mut state = self.lock();
            let now = Instant::now();
            let old: Vec<Point> = state
                .tool
                .iter()
                .filter(|(_, (at, _))| now.saturating_duration_since(*at) >= idle)
                .map(|(point, _)| *point)
                .collect();
            old.into_iter()
                .filter_map(|point| state.tool.remove(&point).map(|(_, l)| (point, l)))
                .collect()
        };
        let points: Vec<Point> = expired.iter().map(|(p, _)| *p).collect();
        self.release(&points);
    }

    /// Keep `point` watched for `bacnet_watch`, marking it asked for now.
    pub(crate) fn tool_watch<T: TransportPort + 'static>(
        &self,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<(), &'static str> {
        let now = Instant::now();
        if let Some((at, _)) = self.lock().tool.get_mut(&point) {
            *at = now;
            return Ok(());
        }
        let lease = self.take(ctx, point)?;
        let mut state = self.lock();
        if state.closed {
            // The session ended meanwhile; the lease goes after the lock.
            drop(state);
            self.release(&[point]);
            return Err(SHUTTING_DOWN);
        }
        if let Some((at, _)) = state.tool.get_mut(&point) {
            // Another call took it meanwhile: keep theirs.
            *at = now;
            drop(state);
            self.release(&[point]);
            return Ok(());
        }
        state.tool.insert(point, (now, lease));
        Ok(())
    }

    /// The URIs of `requested` this server honours, distinct and in order,
    /// with the leases they need taken and set aside for the stream.
    pub(crate) fn reserve<T: TransportPort + 'static>(
        self: &Arc<Self>,
        ctx: &Ctx<T>,
        requested: &[String],
    ) -> Vec<String> {
        self.prune();
        let mut accepted = Vec::new();
        let mut claim = Claim {
            subs: Arc::clone(self),
            points: Vec::new(),
            leases: Vec::new(),
        };
        let mut seen = HashSet::new();
        for text in requested {
            if !seen.insert(text.as_str()) {
                continue;
            }
            let honoured = match uri::parse(text) {
                Ok(Uri::Point(point)) if ctx.reach_known(point.device) => {
                    match self.take(ctx, point) {
                        Ok(lease) => {
                            claim.points.push(point);
                            claim.leases.push((text.clone(), lease));
                            true
                        }
                        Err(_) => false,
                    }
                }
                Ok(Uri::Devices | Uri::DevicesAfter(_) | Uri::Alarms) => ctx.hub.is_some(),
                Ok(Uri::DeviceAlarms(device)) => ctx.hub.is_some() && ctx.reach_known(device),
                Ok(Uri::Session | Uri::Ledger) => true,
                // A device summary has no updates; anything else isn't ours.
                Ok(Uri::Device(_) | Uri::Point(_)) | Err(_) => false,
            };
            if honoured {
                accepted.push(text.clone());
            }
        }
        if !accepted.is_empty() {
            let mut state = self.lock();
            if state.closed {
                // The session ended meanwhile: honour nothing, and let the
                // claim's leases go after the lock.
                drop(state);
                drop(claim);
                return Vec::new();
            }
            let mut oldest = Vec::new();
            while state.pending.len() >= MAX_PENDING {
                oldest.push(state.pending.remove(0));
            }
            state.pending.push(Pending {
                at: Instant::now(),
                uris: accepted.clone(),
                claim,
            });
            drop(state);
            // Dropped after the lock: their leases' release takes it.
            drop(oldest);
        }
        accepted
    }

    /// The leases reserved for a stream acknowledged with `uris`.
    pub(crate) fn claim(&self, uris: &[String]) -> Option<Claim> {
        let mut state = self.lock();
        let at = state.pending.iter().position(|p| p.uris == uris)?;
        Some(state.pending.remove(at).claim)
    }

    /// Drop reservations no stream claimed in time.
    pub(crate) fn prune(&self) {
        let stale: Vec<Pending> = {
            let mut state = self.lock();
            let now = Instant::now();
            let (old, kept) = std::mem::take(&mut state.pending)
                .into_iter()
                .partition(|p| now.saturating_duration_since(p.at) >= UNCLAIMED);
            state.pending = kept;
            old
        };
        drop(stale);
    }

    /// Drop every reservation, legacy subscription and tool watch, when
    /// the session ends.
    pub(crate) fn clear(&self) {
        let (pending, legacy, tool) = {
            let mut state = self.lock();
            state.closed = true;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.legacy),
                std::mem::take(&mut state.tool),
            )
        };
        drop(pending);
        if !legacy.is_empty() {
            drop(legacy);
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        let points: Vec<Point> = tool.keys().copied().collect();
        drop(tool);
        self.release(&points);
    }
}

/// The accepted URIs of a stream, distinct, in the order the
/// acknowledgment lists them.
fn accepted(context: &SubscriptionContext) -> Vec<String> {
    let mut seen = HashSet::new();
    context
        .accepted()
        .resource_subscriptions
        .iter()
        .flatten()
        .filter(|uri| seen.insert(uri.as_str()))
        .cloned()
        .collect()
}

/// What one subscriber follows: its points' change signals, the
/// discovery table's events and the alarm tick. A listen stream's set is
/// fixed; a legacy session's follows its subscribes and unsubscribes
/// ([`Follow::set`]). [`Follow::next`] waits on watch and broadcast
/// channels and timers only, never a lock (#1498), and is cancel safe.
pub(crate) struct Follow {
    points: Vec<(String, Changes)>,
    devices: Option<broadcast::Receiver<DeviceEvent>>,
    devices_due: Option<Instant>,
    devices_last: Option<Instant>,
    device_uris: Vec<String>,
    alarm_uris: Vec<String>,
    alarms: Interval,
}

/// Why [`Follow::next`] woke.
pub(crate) enum Wake {
    /// This resource changed.
    Updated(String),
    /// A device arrived or was lost: every device-list page followed
    /// ([`Follow::device_uris`]) changed, since each one counts `total`.
    Devices,
    /// The alarm resources are due for a poll.
    Alarms,
}

impl Follow {
    pub(crate) fn new() -> Self {
        let mut alarms = tokio::time::interval_at(Instant::now() + ALARMS_EVERY, ALARMS_EVERY);
        alarms.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            points: Vec::new(),
            devices: None,
            devices_due: None,
            devices_last: None,
            device_uris: Vec::new(),
            alarm_uris: Vec::new(),
            alarms,
        }
    }

    /// Follow `uris`, each point URI with its change signal. A point
    /// already followed on the same watch keeps the signal it had, so
    /// nothing is announced twice.
    /// `engine` gives the discovery table's events, when the device list
    /// is followed.
    pub(crate) fn set<T: TransportPort + 'static>(
        &mut self,
        engine: Option<&Engine<T>>,
        uris: Vec<(String, Option<Changes>)>,
    ) {
        let mut kept: HashMap<String, Changes> =
            std::mem::take(&mut self.points).into_iter().collect();
        self.device_uris.clear();
        self.alarm_uris.clear();
        for (name, changes) in uris {
            if let Some(changes) = changes {
                // The old signal only while it is the same watch: a URI
                // unsubscribed and subscribed again between two calls has a
                // new one, and the old one never fires again.
                let changes = match kept.remove(&name) {
                    Some(old) if old.same(&changes) => old,
                    _ => changes,
                };
                self.points.push((name, changes));
            } else {
                match uri::parse(&name) {
                    Ok(Uri::Devices | Uri::DevicesAfter(_)) => self.device_uris.push(name),
                    Ok(Uri::Alarms | Uri::DeviceAlarms(_)) => self.alarm_uris.push(name),
                    _ => {}
                }
            }
        }
        if self.device_uris.is_empty() {
            self.devices = None;
            self.devices_due = None;
        } else if self.devices.is_none() {
            self.devices = engine.map(Engine::device_events);
        }
    }

    /// The signal followed for `name`, for tests.
    #[cfg(test)]
    pub(crate) fn signal(&self, name: &str) -> Option<&Changes> {
        self.points
            .iter()
            .find(|(followed, _)| followed == name)
            .map(|(_, changes)| changes)
    }

    /// Whether nothing followed can change.
    pub(crate) fn quiet(&self) -> bool {
        self.points.is_empty() && self.devices.is_none() && self.alarm_uris.is_empty()
    }

    /// The device-list pages followed, the first and any later ones.
    pub(crate) fn device_uris(&self) -> &[String] {
        &self.device_uris
    }

    /// The alarm resources followed.
    pub(crate) fn alarm_uris(&self) -> &[String] {
        &self.alarm_uris
    }

    /// The next change to announce, or the alarm tick; pending while
    /// nothing followed can change.
    pub(crate) async fn next(&mut self) -> Wake {
        loop {
            tokio::select! {
                biased;
                at = point_change(&mut self.points), if !self.points.is_empty() => {
                    return Wake::Updated(self.points[at].0.clone());
                }
                change = device_change(&mut self.devices), if self.devices.is_some() => {
                    match change {
                        None => self.devices = None,
                        Some(true) if self.devices_due.is_none() => {
                            let earliest = self
                                .devices_last
                                .map_or_else(Instant::now, |at| at + DEVICES_EVERY);
                            self.devices_due = Some(earliest.max(Instant::now()));
                        }
                        Some(_) => {}
                    }
                }
                () = tokio::time::sleep_until(self.devices_due.unwrap_or_else(Instant::now)),
                    if self.devices_due.is_some() => {
                    self.devices_due = None;
                    self.devices_last = Some(Instant::now());
                    return Wake::Devices;
                }
                _ = self.alarms.tick(), if !self.alarm_uris.is_empty() => return Wake::Alarms,
                else => std::future::pending::<()>().await,
            }
        }
    }
}

/// The index of the next point to change. Each signal's wait is cancel
/// safe, so the ones not first lose nothing.
async fn point_change(points: &mut [(String, Changes)]) -> usize {
    let mut waiting: FuturesUnordered<_> = points
        .iter_mut()
        .enumerate()
        .map(|(at, (_, changes))| async move {
            changes.changed().await;
            at
        })
        .collect();
    match waiting.next().await {
        Some(at) => at,
        None => std::future::pending().await,
    }
}

/// Serve one stream until the client cancels it or the server closes.
/// `Ok` on a graceful close, which sends the stream's final result.
pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    context: SubscriptionContext,
) -> Result<(), ErrorData> {
    let _listening = ctx.listening();
    let uris = accepted(&context);
    // Held to the end: its leases keep the points watched.
    let claim = ctx.subs.claim(&uris);
    if claim.is_none() && !uris.is_empty() {
        // Only when the reservation was dropped before the stream began.
        tracing::warn!(
            "a listen stream found no watches set aside for it; its points won't update"
        );
    }
    let leases = claim.as_ref().map_or(&[][..], |c| &c.leases[..]);
    let mut follow = Follow::new();
    follow.set(
        ctx.engine.as_deref(),
        uris.iter()
            .map(|name| {
                let changes = leases
                    .iter()
                    .find(|(held, _)| held == name)
                    .map(|(_, lease)| lease.changes());
                (name.clone(), changes)
            })
            .collect(),
    );
    let sink = context.sink().clone();
    tracing::debug!(
        streams = uris.len(),
        quiet = follow.quiet(),
        "listen stream opened"
    );
    loop {
        let wake = tokio::select! {
            biased;
            () = context.cancelled() => return Ok(()),
            () = ctx.closing.cancelled() => return Ok(()),
            wake = follow.next() => wake,
        };
        match wake {
            Wake::Updated(name) => {
                if sink.notify_resource_updated(name).await.is_err() {
                    return Ok(());
                }
            }
            Wake::Devices => {
                for name in follow.device_uris() {
                    if sink.notify_resource_updated(name.as_str()).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Wake::Alarms => {
                for name in follow.alarm_uris() {
                    if crate::resources::refresh_alarms(ctx, name).await
                        && sink.notify_resource_updated(name.as_str()).await.is_err()
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Whether the discovery table's next event is an arrival or a loss (a
/// lost receiver position counts, since one may have been missed); `None`
/// once the channel is closed.
async fn device_change(events: &mut Option<broadcast::Receiver<DeviceEvent>>) -> Option<bool> {
    match events.as_mut()?.recv().await {
        Ok(event) => Some(!matches!(event.kind, DeviceEventKind::Updated)),
        Err(RecvError::Lagged(_)) => Some(true),
        Err(RecvError::Closed) => None,
    }
}
