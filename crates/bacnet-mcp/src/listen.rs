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
//! The listen loop waits on the leases' signals, the discovery table's
//! events and an alarm tick. Its arms send notifications and refresh
//! alarms; the futures it leaves pending meanwhile wait on watch and
//! broadcast channels only, never a lock (#1498).

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_client::client::{DeviceEvent, DeviceEventKind};
use bacnet_ops::watch::{Lease, Point};
use bacnet_transport::port::TransportPort;
use futures_util::stream::{FuturesUnordered, StreamExt};
use rmcp::ErrorData;
use rmcp::service::SubscriptionContext;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;

use crate::context::Ctx;
use crate::resources::uri::{self, DEVICES, LEDGER, SESSION, Uri};

/// How long a reservation waits for its stream.
const UNCLAIMED: Duration = Duration::from_secs(30);

/// The fastest the device list is announced as changed.
const DEVICES_EVERY: Duration = Duration::from_secs(1);

/// How long a `bacnet_watch` point stays watched without being asked for:
/// as long as its cursor lasts.
pub(crate) const TOOL_IDLE: Duration = Duration::from_secs(600);

/// How often subscribed alarm resources are polled.
pub(crate) const ALARMS_EVERY: Duration = Duration::from_secs(60);

/// What the principal watches, through listen streams and `bacnet_watch`.
#[derive(Default)]
pub(crate) struct Subs {
    state: Mutex<SubsState>,
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
        if !state.held.contains_key(&point) && state.held.len() >= ctx.policy.caps.watches {
            return Err("principal-cap");
        }
        let lease = hub.watch(point).map_err(|_| "process-cap")?;
        *state.held.entry(point).or_default() += 1;
        Ok(lease)
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
                Ok(Uri::Devices | Uri::Alarms) => ctx.hub.is_some(),
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
            self.lock().pending.push(Pending {
                at: Instant::now(),
                uris: accepted.clone(),
                claim,
            });
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
    fn prune(&self) {
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

    /// Drop every reservation and tool watch, at shutdown.
    pub(crate) fn clear(&self) {
        let (pending, tool) = {
            let mut state = self.lock();
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.tool),
            )
        };
        drop(pending);
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

type Changed = Pin<Box<dyn std::future::Future<Output = (usize, Lease)> + Send>>;

fn changed(at: usize, mut lease: Lease) -> Changed {
    Box::pin(async move {
        lease.changed().await;
        (at, lease)
    })
}

/// Serve one stream until the client cancels it or the server closes.
/// `Ok` on a graceful close, which sends the stream's final result.
pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    context: SubscriptionContext,
) -> Result<(), ErrorData> {
    let _listening = ctx.listening();
    let uris = accepted(&context);
    let mut claim = ctx.subs.claim(&uris);
    if claim.is_none() && !uris.is_empty() {
        // Only when the reservation was dropped before the stream began.
        tracing::warn!(
            "a listen stream found no watches set aside for it; its points won't update"
        );
    }
    let mut names = Vec::new();
    let mut points: FuturesUnordered<Changed> = FuturesUnordered::new();
    if let Some(claim) = claim.as_mut() {
        for (at, (name, lease)) in std::mem::take(&mut claim.leases).into_iter().enumerate() {
            names.push(name);
            points.push(changed(at, lease));
        }
    }
    let alarm_uris: Vec<&String> = uris
        .iter()
        .filter(|u| matches!(uri::parse(u), Ok(Uri::Alarms | Uri::DeviceAlarms(_))))
        .collect();
    let mut devices = uris
        .iter()
        .any(|u| u == DEVICES)
        .then(|| ctx.engine.as_ref().map(|e| e.device_events()))
        .flatten();
    let mut devices_due: Option<Instant> = None;
    let mut devices_last: Option<Instant> = None;
    let mut alarms = tokio::time::interval_at(Instant::now() + ALARMS_EVERY, ALARMS_EVERY);
    alarms.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let sink = context.sink().clone();
    let quiet = uris.iter().all(|u| u == SESSION || u == LEDGER);
    tracing::debug!(streams = uris.len(), quiet, "listen stream opened");
    loop {
        let update = tokio::select! {
            biased;
            () = context.cancelled() => return Ok(()),
            () = ctx.closing.cancelled() => return Ok(()),
            Some((at, lease)) = points.next(), if !points.is_empty() => {
                points.push(changed(at, lease));
                names[at].clone()
            }
            change = device_change(&mut devices), if devices.is_some() => {
                match change {
                    None => devices = None,
                    Some(true) if devices_due.is_none() => {
                        let earliest = devices_last.map_or_else(Instant::now, |at| at + DEVICES_EVERY);
                        devices_due = Some(earliest.max(Instant::now()));
                    }
                    Some(_) => {}
                }
                continue;
            }
            () = tokio::time::sleep_until(devices_due.unwrap_or_else(Instant::now)), if devices_due.is_some() => {
                devices_due = None;
                devices_last = Some(Instant::now());
                DEVICES.to_string()
            }
            _ = alarms.tick(), if !alarm_uris.is_empty() => {
                for name in &alarm_uris {
                    if crate::resources::refresh_alarms(ctx, name).await
                        && sink.notify_resource_updated(name.as_str()).await.is_err()
                    {
                        return Ok(());
                    }
                }
                continue;
            }
        };
        if sink.notify_resource_updated(update).await.is_err() {
            return Ok(());
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
