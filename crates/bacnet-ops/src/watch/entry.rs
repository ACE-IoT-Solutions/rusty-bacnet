//! One watched point's task: its first read, the managed subscription or
//! the polls, and the cancel at the end.
//!
//! Each request is raced against the entry's stop, so a stop never waits
//! for a poll; a subscription request dropped that way may have reached
//! the device, so the cancel goes anyway. Every request takes its turn at
//! the device's gate in the engine's limits, as the TUI's watch jobs do,
//! and the first ones run under the hub's account. No lock is held across
//! an await.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{
    CovPropertySubscription, CovTarget, ManagedCOVSubscription, ManagedCOVSubscriptionEvent,
    ManagedCOVSubscriptionOptions,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;

use super::{Inner, POINT_PROPERTIES, Registry, Shared, Source, State, Watched};
use crate::engine::{Wanted, network_of, unreachable};
use crate::format::{RawRow, RowError};
use crate::limits::charged;

/// The longest wait between polls of a device that stopped answering.
const LONGEST_BACKOFF: Duration = Duration::from_secs(60);

/// How long one cancel may take before it is given up.
const CANCEL_WAIT: Duration = Duration::from_secs(30);

/// Counts the entry's task out of the hub's running tasks when it ends,
/// however it ends.
struct Running(Arc<Registry>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0
            .tasks
            .send_modify(|tasks| *tasks = tasks.saturating_sub(1));
    }
}

/// Run `shared`'s watch until it is stopped, then cancel what it holds.
pub(super) async fn run<T: TransportPort + 'static>(inner: Arc<Inner<T>>, shared: Arc<Shared>) {
    let _running = Running(Arc::clone(&inner.registry));
    let entry = Entry {
        inner: &inner,
        shared: &shared,
        managed: None,
        events: None,
        maybe_subscribed: false,
        target: None,
        next_poll: None,
        retry_cov: None,
        backoff: Duration::ZERO,
    };
    let mut entry = charged(Arc::clone(&inner.account), entry.watch()).await;
    entry.cancel().await;
    let seq = &inner.registry.seq;
    let note = shared.lock().snapshot.note.clone();
    shared.set_state(seq, None, State::Stopped, note);
}

struct Entry<'a, T: TransportPort + 'static> {
    inner: &'a Inner<T>,
    shared: &'a Shared,
    managed: Option<ManagedCOVSubscription>,
    events: Option<broadcast::Receiver<ManagedCOVSubscriptionEvent>>,
    /// A subscription may exist on the device though none is managed: a
    /// request dropped or timed out on its way, or a renewal that failed.
    maybe_subscribed: bool,
    target: Option<CovTarget>,
    next_poll: Option<Instant>,
    retry_cov: Option<Instant>,
    backoff: Duration,
}

impl<'a, T: TransportPort + 'static> Entry<'a, T> {
    /// `work`, unless the entry is stopped first.
    async fn or_stop<F: Future>(&self, work: F) -> Option<F::Output> {
        tokio::select! {
            biased;
            () = self.shared.stopped() => None,
            done = work => Some(done),
        }
    }

    fn device(&self) -> u32 {
        self.shared.point.device
    }

    /// Record values read and signal them: the first read (`poll`
    /// false), which leaves the source to the subscription that follows,
    /// or a poll.
    fn read_done(&self, rows: Vec<RawRow>, poll: bool) {
        let mut state = self.shared.lock();
        state.snapshot.rows = rows;
        state.snapshot.updated = Some(Instant::now());
        state.snapshot.state = State::Live;
        if poll {
            state.snapshot.polls += 1;
            state.snapshot.source = Some(Source::Poll);
        }
        self.shared
            .changed(&mut state, &self.inner.registry.seq, Duration::ZERO);
    }

    fn set_state(&self, source: Option<Source>, state: State, note: Option<String>) {
        self.shared
            .set_state(&self.inner.registry.seq, source, state, note);
    }

    /// Watch until stopped: start, then follow the subscription or poll.
    async fn watch(mut self) -> Self {
        if self.start().await {
            while self.step().await {}
        }
        self
    }

    /// Look the device up, read the point once and subscribe. False when
    /// stopped meanwhile.
    async fn start(&mut self) -> bool {
        let engine = &self.inner.engine;
        let Some(found) = self.or_stop(engine.device(self.device())).await else {
            return false;
        };
        let Some(device) = found else {
            self.set_state(
                Some(Source::Poll),
                State::Unreachable,
                Some(format!(
                    "device {} is not in the device table",
                    self.device()
                )),
            );
            self.poll_later(true);
            return true;
        };
        let target = CovTarget::of_device(&device);
        self.shared.lock().target = Some(target.clone());
        self.target = Some(target);
        let Some(first) = self.or_stop(self.read()).await else {
            return false;
        };
        match first {
            Ok(rows) => self.read_done(rows, false),
            Err(error) if unreachable(&error) => {
                // Subscribing would only time out too.
                self.set_state(
                    Some(Source::Poll),
                    State::Unreachable,
                    Some(error.to_string()),
                );
                self.poll_later(true);
                self.retry_cov = Some(Instant::now() + self.inner.options.retry_cov);
                return true;
            }
            Err(error) => self.set_state(None, State::Failed, Some(error.to_string())),
        }
        self.subscribe().await
    }

    /// Ask for the managed subscription. False when stopped meanwhile.
    async fn subscribe(&mut self) -> bool {
        let Some(target) = self.target.clone() else {
            self.poll_later(false);
            return true;
        };
        // Until the answer comes, the request may have reached the device.
        self.maybe_subscribed = true;
        let Some(result) = self.or_stop(self.request_subscription(target)).await else {
            return false;
        };
        match result {
            Ok(managed) => {
                self.events = Some(managed.events());
                self.managed = Some(managed);
                self.maybe_subscribed = false;
                self.retry_cov = None;
                self.set_state(Some(Source::Cov), State::Live, None);
            }
            Err(error) => {
                let transient = unreachable(&error);
                self.maybe_subscribed = transient;
                let note = format!("COV refused: {error}");
                let state = if transient {
                    State::Unreachable
                } else {
                    self.shared.lock().snapshot.state
                };
                self.set_state(Some(Source::Poll), state, Some(note));
                self.retry_cov = transient.then(|| Instant::now() + self.inner.options.retry_cov);
                self.poll_later(transient);
            }
        }
        true
    }

    async fn request_subscription(
        &self,
        target: CovTarget,
    ) -> Result<ManagedCOVSubscription, Error> {
        let engine = &self.inner.engine;
        let options = &self.inner.options;
        let client = engine.client();
        let limits = engine.limits();
        let device = self.device();
        let network = network_of(client, limits, device).await;
        let managed = ManagedCOVSubscriptionOptions::default()
            .with_renewal_margin(options.renewal_margin)
            .with_renewal_gate(limits.gate(device));
        // The first request goes under this turn; renewals take the gate
        // themselves.
        let _turn = limits.turn_on(device, network).await;
        let point = self.shared.point;
        let process = self.shared.process;
        match point.watched {
            Watched::Object => {
                Arc::clone(client)
                    .manage_cov_subscription(
                        target,
                        process,
                        point.object,
                        false,
                        options.lifetime.secs(),
                        managed,
                    )
                    .await
            }
            Watched::Property { property, index } => {
                let subscription = CovPropertySubscription {
                    subscriber_process_identifier: process,
                    monitored_object_identifier: point.object,
                    monitored_property_identifier: property,
                    monitored_property_array_index: index,
                    confirmed: false,
                    lifetime: options.lifetime.0,
                    cov_increment: None,
                };
                Arc::clone(client)
                    .manage_cov_property_subscription(target, subscription, managed)
                    .await
            }
        }
    }

    /// The point's properties, read once: the object's preset by RPM (or
    /// ReadProperty each), or the one property. A property the device
    /// can't give is an error row.
    async fn read(&self) -> Result<Vec<RawRow>, Error> {
        let engine = &self.inner.engine;
        let point = self.shared.point;
        match point.watched {
            Watched::Object => engine
                .read(point.device, point.object, Wanted::List(&POINT_PROPERTIES))
                .await
                .map(|read| read.rows),
            Watched::Property { property, index } => {
                match engine
                    .read_property(point.device, point.object, property, index)
                    .await
                {
                    Ok(data) => Ok(vec![(property, Ok(data))]),
                    Err(error) if unreachable(&error) => Err(error),
                    Err(error) => Ok(vec![(property, Err(RowError::from(error)))]),
                }
            }
        }
    }

    /// Schedule the next poll: after the poll interval, or, for a device
    /// that didn't answer, after a backoff that doubles up to a minute.
    fn poll_later(&mut self, silent: bool) {
        let every = self.inner.options.poll_every;
        let wait = if silent {
            self.backoff = if self.backoff.is_zero() {
                every
            } else {
                (self.backoff * 2).min(LONGEST_BACKOFF.max(every))
            };
            self.backoff
        } else {
            self.backoff = Duration::ZERO;
            every
        };
        self.next_poll = Some(Instant::now() + wait);
    }

    /// Wait for the next thing to do and do it. False once stopped.
    async fn step(&mut self) -> bool {
        if let Some(events) = &mut self.events {
            let event = tokio::select! {
                biased;
                () = self.shared.stopped() => return false,
                event = events.recv() => event,
            };
            match event {
                Ok(ManagedCOVSubscriptionEvent::Renewed { .. }) => {
                    self.shared.lock().snapshot.renewals += 1;
                }
                Ok(ManagedCOVSubscriptionEvent::RenewalFailed { error }) => {
                    self.lapse(format!("COV renewal failed: {error}")).await;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => self.lapse("COV renewal stopped".into()).await,
            }
            return true;
        }
        let due = match (self.next_poll, self.retry_cov) {
            (Some(poll), Some(retry)) => poll.min(retry),
            (Some(at), None) | (None, Some(at)) => at,
            (None, None) => Instant::now() + self.inner.options.poll_every,
        };
        tokio::select! {
            biased;
            () = self.shared.stopped() => return false,
            () = tokio::time::sleep_until(due) => {}
        }
        if self.retry_cov.is_some_and(|at| at <= Instant::now()) {
            self.retry_cov = None;
            return self.subscribe().await;
        }
        self.poll().await
    }

    /// The subscription ended: poll from now, and subscribe again later.
    async fn lapse(&mut self, why: String) {
        self.events = None;
        if let Some(managed) = self.managed.take() {
            managed.stop().await;
        }
        // The failed renewal may have reached the device.
        self.maybe_subscribed = true;
        self.set_state(Some(Source::Poll), State::Unreachable, Some(why));
        self.next_poll = Some(Instant::now());
        self.retry_cov = Some(Instant::now() + self.inner.options.retry_cov);
    }

    /// Poll the point once. False when stopped meanwhile.
    async fn poll(&mut self) -> bool {
        let Some(read) = self.or_stop(self.read()).await else {
            return false;
        };
        match read {
            Ok(rows) => {
                // The note keeps saying why the point is polled.
                self.read_done(rows, true);
                self.poll_later(false);
            }
            Err(error) if unreachable(&error) => {
                self.set_state(
                    Some(Source::Poll),
                    State::Unreachable,
                    Some(error.to_string()),
                );
                self.poll_later(true);
            }
            Err(error) => {
                self.set_state(Some(Source::Poll), State::Failed, Some(error.to_string()));
                self.poll_later(false);
            }
        }
        true
    }

    /// Cancel the subscription, if there may be one: stop renewing it, then
    /// send a subscription with neither a lifetime nor the confirmed flag,
    /// which the device takes as a cancellation, the way the subscription
    /// went. It takes the device's turn first, so no renewal is in flight
    /// when the renewals stop. Not charged to the hub's bucket.
    async fn cancel(&mut self) {
        let managed = self.managed.take();
        self.events = None;
        if managed.is_none() && !self.maybe_subscribed {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        let engine = &self.inner.engine;
        let client = engine.client();
        let limits = engine.limits();
        let point = self.shared.point;
        let process = self.shared.process;
        let cancel = async {
            let network = network_of(client, limits, point.device).await;
            let _turn = limits.turn_on(point.device, network).await;
            if let Some(managed) = managed {
                managed.stop().await;
            }
            match point.watched {
                Watched::Object => {
                    client
                        .unsubscribe_cov_target(&target, process, point.object)
                        .await
                }
                Watched::Property { property, index } => {
                    client
                        .unsubscribe_cov_property_target(
                            &target,
                            process,
                            point.object,
                            property,
                            index,
                        )
                        .await
                }
            }
        };
        match tokio::time::timeout(CANCEL_WAIT, cancel).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::debug!(
                "watch of {:?}: the cancel failed ({error}); it ends with its lifetime",
                point
            ),
            Err(_) => tracing::debug!(
                "watch of {:?}: the cancel took too long; it ends with its lifetime",
                point
            ),
        }
    }
}
