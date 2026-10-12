//! The hosted B/IP node: [`bacnet_bip_core::node`] on Tokio (#1622).
//!
//! The engine's [`Link`] decides every receive and send without a lock: the
//! receive loop and the send paths read one copy. Its [`Node`] holds this
//! node's own management exchanges and a foreign device's registration,
//! behind a std mutex that only a response, a foreign device's broadcast, an
//! exchange and the driver's timer take, and never across an await. After
//! every call the node's actions are drained under that lock: an exchange
//! that timed out fails its waiter at once, and a send the node asks for (a
//! registration attempt, or the broadcasts it held for one) waits for the
//! driver task, which makes every such send. A call made where nothing can
//! be awaited, as when a request is dropped, so never has to send.
//!
//! Two clocks, which never meet: the node runs on Tokio time from the start
//! ([`HostClock`]), which paused-time tests rely on, and the BBMD on std
//! time (`bbmd/clock.rs`), as before. No time on one is compared with a
//! time on the other, and the management rate limit keeps its own std
//! clock, as before.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_bip_codec::{BvlcFunction, Bvll, BvllError};
use bacnet_bip_core::node::{
    Action, Admission, ConfigError, ExchangeId, Inbound, Link, LocalFailure, Mode, Node, Policy,
    RegistrationEvent, Response, SendRefusal,
};
use bacnet_bip_core::{Duration as CoreDuration, Instant as CoreInstant};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::{oneshot, Notify};
use tracing::debug;

use super::client_snapshot::ClientRecord;
use super::rate_limit::ManagementRateLimiter;
use super::BipSocket;
use crate::bbmd;
use crate::bvll::{bvll_error, encode_bvll, BvllMessage};

/// Exchange slots: one, which the management helpers and a foreign
/// device's registration share, as before the engine.
const SLOTS: usize = 1;

/// The engine as the hosted transport runs it.
pub(super) type HostNode = Node<SLOTS>;

/// Tokio time mapped onto the engine's nanosecond timeline, from the
/// moment the node started. Every call into the node reads it, so the
/// engine's deadlines are Tokio deadlines and paused time drives them.
#[derive(Debug, Clone, Copy)]
pub(super) struct HostClock {
    origin: tokio::time::Instant,
}

impl HostClock {
    fn new() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> CoreInstant {
        let elapsed = tokio::time::Instant::now().saturating_duration_since(self.origin);
        // Saturates some 584 years after the node started.
        CoreInstant::ZERO + CoreDuration::saturating_from_core(elapsed)
    }

    /// The Tokio instant of an engine time. Both ways are exact, so a timer
    /// set here never wakes before the engine's deadline is due.
    fn instant(&self, at: CoreInstant) -> tokio::time::Instant {
        let offset = Duration::from_nanos(at.as_nanos());
        self.origin
            .checked_add(offset)
            .unwrap_or_else(|| self.origin + Duration::from_secs(86_400 * 365))
    }
}

/// The host's rules [`Link::receive`] consults: the management rate limit,
/// and the size limit on a table ACK.
pub(super) struct HostPolicy<'a> {
    limiter: &'a Mutex<ManagementRateLimiter>,
    /// Why [`accept_table`](Policy::accept_table) refused an ACK, which
    /// the request waiting for it fails with.
    refusal: Option<Error>,
}

impl<'a> HostPolicy<'a> {
    pub(super) fn new(limiter: &'a Mutex<ManagementRateLimiter>) -> Self {
        Self {
            limiter,
            refusal: None,
        }
    }

    /// Why the policy refused a table ACK, if it did.
    pub(super) fn into_refusal(self) -> Option<Error> {
        self.refusal
    }
}

impl Policy for HostPolicy<'_> {
    fn admit_management(
        &mut self,
        source: SocketAddrV4,
        _function: BvlcFunction,
        _now: CoreInstant,
    ) -> bool {
        let mut limiter = self.limiter.lock().unwrap_or_else(PoisonError::into_inner);
        limiter.check_now(source.ip().octets())
    }

    fn accept_table(&mut self, ack: &Bvll<'_>) -> bool {
        let checked = match *ack {
            Bvll::ReadBroadcastDistributionTableAck(table) => bbmd::bdt_from_wire(table).map(drop),
            Bvll::ReadForeignDeviceTableAck(table) => bbmd::fdt_from_wire(table).map(drop),
            _ => Ok(()),
        };
        match checked {
            Ok(()) => true,
            Err(error) => {
                self.refusal = Some(error);
                false
            }
        }
    }
}

/// What a manual request gets: the response, or why there is none.
pub(super) type Outcome = Result<BvllMessage, Error>;

/// A manual request waiting for its response.
pub(super) struct Waiter {
    pub(super) tx: oneshot::Sender<Outcome>,
    /// For the error a timeout reports.
    pub(super) timeout: Duration,
    /// A response that beat the request's send, kept until the node says
    /// the send settles it.
    pub(super) early: Option<Outcome>,
}

/// A registration attempt the node asked to send.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    exchange: ExchangeId,
    to: SocketAddrV4,
    ttl: u16,
}

/// What the node lock guards.
pub(super) struct NodeState {
    pub(super) node: HostNode,
    /// Manual requests, by exchange.
    pub(super) waiters: HashMap<ExchangeId, Waiter>,
    /// A registration attempt for the driver to send.
    register: Option<Attempt>,
    /// When the automatic attempt in flight started.
    attempt_started: Option<CoreInstant>,
    /// Broadcasts held for the first registration.
    held: VecDeque<Bytes>,
    /// Held broadcasts released, for the driver to send.
    release: Vec<Bytes>,
    /// Set by stop: no request starts, and nothing more is counted.
    pub(super) stopped: bool,
}

impl NodeState {
    /// Do what the node asks, and add its counts to `record`.
    fn drain(&mut self, now: CoreInstant, record: &ClientRecord) {
        loop {
            match self.node.poll_action() {
                Action::Idle => break,
                Action::Register { exchange, to, ttl } => {
                    self.attempt_started = Some(now);
                    self.register = Some(Attempt { exchange, to, ttl });
                }
                Action::ExchangeEnded { exchange } => {
                    if let Some(waiter) = self.waiters.remove(&exchange) {
                        let _ = waiter.tx.send(Err(Error::Timeout(waiter.timeout)));
                    }
                }
                Action::ReleaseHeld => self.release.extend(self.held.drain(..)),
                Action::DiscardHeld => self.held.clear(),
                Action::Registration(event) => log_registration(event),
            }
        }
        let counters = self.node.take_counters();
        if !self.stopped {
            record.absorb(&counters);
        }
    }

    /// Hand `exchange`'s waiter its outcome, if it still waits.
    pub(super) fn complete(&mut self, exchange: ExchangeId, outcome: Outcome) {
        if let Some(waiter) = self.waiters.remove(&exchange) {
            let _ = waiter.tx.send(outcome);
        }
    }
}

fn log_registration(event: RegistrationEvent) {
    match event {
        RegistrationEvent::Unanswered => {
            debug!("Foreign registration attempt did not return a matched result");
        }
        event => debug!(?event, "Foreign registration"),
    }
}

/// The node of one started transport, shared by its receive loop, its send
/// paths, its driver task and the requests in flight.
pub(super) struct NodeHandle {
    link: Link,
    clock: HostClock,
    socket: Arc<BipSocket>,
    record: Arc<ClientRecord>,
    /// A foreign device's renewal interval.
    renewal: Option<CoreDuration>,
    state: Mutex<NodeState>,
    /// Tells the driver that the node changed.
    wake: Notify,
}

impl NodeHandle {
    /// The node on `link`, sending from `socket` and counting into
    /// `record`. A foreign device's first registration attempt is admitted
    /// at once and sent when the driver first runs, which is also when what
    /// the node counted for it reaches `record`, as with the registration
    /// worker before the engine.
    pub(super) fn new(
        link: Link,
        socket: Arc<BipSocket>,
        record: Arc<ClientRecord>,
    ) -> Result<Self, Error> {
        let clock = HostClock::new();
        let node = HostNode::new(link, clock.now()).map_err(config_error)?;
        let renewal = match link.mode {
            Mode::Foreign(foreign) => Some(foreign.effective_renewal()),
            Mode::Normal | Mode::Bbmd => None,
        };
        let handle = Self {
            link,
            clock,
            socket,
            record,
            renewal,
            state: Mutex::new(NodeState {
                node,
                waiters: HashMap::new(),
                register: None,
                attempt_started: None,
                held: VecDeque::new(),
                release: Vec::new(),
                stopped: false,
            }),
            wake: Notify::new(),
        };
        Ok(handle)
    }

    /// The node's link: its address, subnet and mode.
    pub(super) fn link(&self) -> &Link {
        &self.link
    }

    /// The node's clock.
    pub(super) fn clock(&self) -> HostClock {
        self.clock
    }

    /// The socket every send leaves from.
    pub(super) fn socket(&self) -> &Arc<BipSocket> {
        &self.socket
    }

    fn lock(&self) -> MutexGuard<'_, NodeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Call into the node now, under the lock, then drain what it asks for
    /// and wake the driver.
    pub(super) fn with<R>(&self, call: impl FnOnce(&mut NodeState, CoreInstant) -> R) -> R {
        let mut state = self.lock();
        let now = self.clock.now();
        let result = call(&mut state, now);
        state.drain(now, &self.record);
        drop(state);
        self.wake.notify_one();
        result
    }

    /// Match a response against this node's exchanges, and hand a manual
    /// request its outcome. `frame` is the whole datagram; `refusal` is
    /// why the policy refused a table ACK.
    pub(super) fn resolve(&self, response: Response<'_>, frame: &[u8], refusal: Option<Error>) {
        let function = response.function;
        self.with(|state, now| match state.node.resolve(response, now) {
            Inbound::Matched {
                exchange,
                message,
                accepted,
                settled,
            } => {
                if !state.waiters.contains_key(&exchange) {
                    return;
                }
                let outcome = if accepted {
                    Ok(owned(function, frame))
                } else {
                    Err(refused(message.err(), refusal))
                };
                if settled {
                    state.complete(exchange, outcome);
                } else if let Some(waiter) = state.waiters.get_mut(&exchange) {
                    waiter.early = Some(outcome);
                }
            }
            Inbound::Unmatched { peer, code, late } => {
                debug!(%peer, ?code, late, %function, "Received unmatched BVLC response");
            }
            _ => {}
        });
    }

    /// Whether a foreign device's broadcast of `npdu` goes now. One held
    /// is kept until the node releases or drops it.
    pub(super) fn admit_broadcast(&self, npdu: &[u8]) -> Result<Admission, Error> {
        self.with(|state, now| {
            let admission = state.node.admit_broadcast(now).map_err(send_refusal)?;
            if admission == Admission::Hold {
                state.held.push_back(Bytes::copy_from_slice(npdu));
            }
            Ok(admission)
        })
    }

    /// Time until a foreign device's next automatic registration attempt:
    /// one renewal interval after the last one started.
    pub(super) fn next_attempt_in(&self) -> Option<Duration> {
        let state = self.lock();
        let now = self.clock.now();
        let view = state.node.registration(now)?;
        let next = match view.next_attempt {
            Some(next) => next,
            None => state.attempt_started? + self.renewal?,
        };
        Some(Duration::from_nanos(
            next.saturating_duration_since(now).as_nanos(),
        ))
    }

    /// What this node's exchanges came to, as the transport reports them.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> super::BvlcClientSnapshot {
        let mut snapshot = self.record.snapshot();
        snapshot.foreign_registration.next_attempt_in = self.next_attempt_in();
        snapshot
    }

    /// Stop: requests in flight fail as their channel closes, none starts,
    /// and nothing more is sent or counted.
    pub(super) fn stop(&self) {
        let mut state = self.lock();
        state.stopped = true;
        state.waiters.clear();
        state.register = None;
        state.held.clear();
        state.release.clear();
    }

    /// Send what the node asks for, and wake it at its deadlines, until the
    /// task is aborted.
    pub(super) async fn drive(self: Arc<Self>) {
        // What the node asked for at its start.
        self.with(|_, _| ());
        loop {
            let (attempt, release) = {
                let mut state = self.lock();
                (state.register.take(), std::mem::take(&mut state.release))
            };
            if let Some(attempt) = attempt {
                self.send_attempt(attempt).await;
            }
            for npdu in release {
                self.send_released(&npdu).await;
            }
            let deadline = self.lock().node.next_deadline();
            let timer = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(self.clock.instant(at)).await,
                    None => std::future::pending().await,
                }
            };
            // Neither future holds a lock, and either may be dropped.
            tokio::select! {
                biased;
                () = self.wake.notified() => {}
                () = timer => self.with(|state, now| state.node.on_deadline(now)),
            }
        }
    }

    async fn send_attempt(&self, attempt: Attempt) {
        let mut buf = BytesMut::with_capacity(6);
        let message = Bvll::RegisterForeignDevice { ttl: attempt.ttl };
        let sent = match encode_bvll(&mut buf, &message) {
            Ok(()) => self
                .socket
                .send_to(&buf, attempt.to)
                .await
                .map(drop)
                .map_err(Error::Transport),
            Err(error) => Err(error),
        };
        match sent {
            Ok(()) => {
                self.with(|state, now| state.node.exchange_sent(attempt.exchange, now));
            }
            Err(error) => {
                debug!(bbmd = %attempt.to, %error, "Foreign registration attempt was not sent");
                self.with(|state, now| {
                    let failure = LocalFailure::SendFailed;
                    state.node.exchange_failed(attempt.exchange, failure, now);
                });
            }
        }
    }

    /// Send a broadcast the node held for the first registration.
    async fn send_released(&self, npdu: &[u8]) {
        let Ok((outgoing, _)) = self.link.broadcast(npdu) else {
            return;
        };
        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        if let Err(error) = encode_bvll(&mut buf, &outgoing.message) {
            debug!(%error, "Failed to encode a held broadcast");
            return;
        }
        if let Err(error) = self.socket.send_to(&buf, outgoing.to).await {
            debug!(%error, "Failed to send a held broadcast");
        }
    }
}

/// An owned copy of the response in `frame`, a whole datagram whose header
/// the codec checked.
fn owned(function: BvlcFunction, frame: &[u8]) -> BvllMessage {
    BvllMessage {
        function,
        payload: Bytes::copy_from_slice(
            frame.get(bacnet_bip_codec::HEADER_LENGTH..).unwrap_or(&[]),
        ),
    }
}

/// Why a matched response doesn't answer: the codec's error, or the
/// policy's.
fn refused(decode: Option<BvllError>, policy: Option<Error>) -> Error {
    match (decode, policy) {
        (Some(error), _) => bvll_error(error),
        (None, Some(error)) => error,
        (None, None) => Error::Encoding("BVLC response refused".into()),
    }
}

fn config_error(error: ConfigError) -> Error {
    Error::Encoding(error.to_string())
}

pub(super) fn send_refusal(refusal: SendRefusal) -> Error {
    Error::Encoding(refusal.to_string())
}
