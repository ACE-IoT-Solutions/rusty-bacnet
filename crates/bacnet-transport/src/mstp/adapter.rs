//! The Tokio adapter that drives the portable master core.
//!
//! The transport's task owns the [`MasterCore`], its receive storage and a
//! boxed [`ActionBuffers`]; the core borrows the two buffers for as long as
//! the task's future lives, so they go away with the task, whether it ends
//! or is aborted. The core's queues are shared with the send calls (see the
//! `queues` module).
//!
//! # Input: the octet path
//!
//! Every octet a serial read returns goes to [`MasterCore::on_octet`],
//! stamped with the time that read returned, and the core's own stream
//! decoder assembles the frames: the same receive path the simulator and the
//! fuzzers exercise. A host learns of octets in chunks, after they arrived,
//! so a stamp can only be late. A late stamp moves the line's last activity
//! later, which can only defer this station's own sends and lengthen its
//! silence timers: the safe direction. The reply window the core measures
//! from a late stamp is covered by its reply transmit margin. Chunk gaps
//! inside a frame are covered by a longer T_frame_abort
//! ([`HOST_FRAME_ABORT_MS`](super::HOST_FRAME_ABORT_MS)).
//!
//! # Deadlines
//!
//! The task sleeps until [`MasterCore::next_deadline`], mapped onto Tokio
//! time from the moment the transport started, and stamps
//! [`MasterCore::on_deadline`] with the time it actually woke. A deadline
//! serviced a millisecond or more late counts in the core's
//! `deadline_misses`, published in [`MstpDiagnosticsSnapshot::mac`](super::MstpDiagnosticsSnapshot::mac).
//!
//! # Sending a frame
//!
//! After every call into the core the task polls its actions until none is
//! left. `SetDirection` needs nothing here: every [`SerialPort`] switches
//! its own transceiver as it writes. A handed-over frame is sent in four
//! steps, following the core's transmit contract:
//!
//! 1. Octets already waiting in the port are read without blocking and fed
//!    to the core, stamped with the time of the read. They are line
//!    activity, and may make the core take the frame back.
//! 2. [`MasterCore::on_transmit_started`]; on any refusal the frame is
//!    dropped (a deferred one comes back as a new transmit).
//! 3. [`SerialPort::write`], then [`SerialPort::drain`]. The frame is
//!    complete when the drain returns, but never before its time on the wire
//!    at the configured baud rate has passed since the start: a port without
//!    a drain is timed that way alone, and one whose drain returns early (a
//!    USB adapter still shifting bits out) is held to it. A write or drain
//!    error aborts the frame.
//! 4. On a port with [`Echo::Full`], as many octets as were sent are read
//!    and dropped, within [`ECHO_GRACE`] of the frame's end; octets after
//!    them are kept. Then [`MasterCore::on_transmit_complete`] is stamped
//!    with the frame's end, and the kept octets are fed after it, stamped
//!    with the time of their read.
//!
//! Per port kind: `TokioSerialPort` writes to the driver and drains with
//! `tcdrain` on Unix (on Windows it has no drain, so the wire time is used);
//! its echo is whatever the hardware does, `Echo::None` unless configured.
//! Kernel RS-485 is the same port with the kernel switching RTS around each
//! write. `GpioDirectionPort` drains inside its write and holds the driver
//! for its post-transmit delay before switching back, so the frame
//! completes when its write returns; its echo is the inner port's.
//! [`LoopbackSerial`](super::LoopbackSerial) delivers at once and is timed by
//! the wire time, echoing only when built with [`Echo::Full`].
//!
//! # Answering a request
//!
//! A data-expecting-reply frame for this station goes up with a one-use
//! reply channel. An answer through it goes to
//! [`MasterCore::complete_request`]; a dropped channel asks the core for
//! Reply Postponed. Once the core has finished the request (answered, sent
//! Reply Postponed at its decision deadline, or given up) the channel is
//! closed, so a late answer fails at the sender and the application sends it
//! the ordinary way. An answer that reached the channel too late for the
//! core is queued as an ordinary NPDU to the requester, to go out with the
//! next token, as Clause 9.5.6.9 has a deferred reply do.

use std::future::Future;
use std::ops::Range;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use bacnet_mstp_codec::{FrameType, BROADCAST_MAC, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::{
    Action, ActionBuffers, CoreError, Direction, MasterCore, MasterState, MstpCoreConfig,
    QueueRejectReason, ReplyDecision, RequestId, TransmitFailure, TransmitId,
    DEFAULT_DELIVERY_SLOTS,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use super::diagnostics::{increment, Counters};
use super::queues::{npdu_priority, CoreClock, SharedQueues};
use super::{Echo, SerialPort};
use crate::port::{ReceivedNpdu, TransportProvenance};

/// How long after a frame's end its echo may still be arriving: room for a
/// USB adapter's receive latency timer (16 ms by default on FTDI parts).
pub(super) const ECHO_GRACE: Duration = Duration::from_millis(20);

/// Octets asked for in one serial read.
const READ_CHUNK: usize = 512;

/// Reads at most in the non-blocking drain before a start, so a line that
/// never falls quiet cannot hold the task there.
const DRAIN_READS: usize = 8;

type Core<'a> = MasterCore<'a, SharedQueues>;

/// The transport's task: everything it needs, moved in at start.
pub(super) struct Task<S> {
    pub(super) serial: S,
    pub(super) config: MstpCoreConfig,
    pub(super) queues: SharedQueues,
    pub(super) clock: CoreClock,
    pub(super) npdus: mpsc::Sender<ReceivedNpdu>,
    pub(super) counts: Arc<Counters>,
}

impl<S: SerialPort> Task<S> {
    /// Run the core until the serial port fails or the task is aborted.
    pub(super) async fn run(self) {
        let Task {
            serial,
            config,
            queues,
            clock,
            npdus,
            counts,
        } = self;
        let _closed = CloseOnExit(queues.clone());
        let mut rx_storage = vec![0u8; MAX_STANDARD_FRAME_LENGTH];
        let mut buffers = Box::new(ActionBuffers::<DEFAULT_DELIVERY_SLOTS>::new());
        let mut core =
            match MasterCore::new(config, queues, &mut rx_storage, &mut buffers, clock.now()) {
                Ok(core) => core,
                Err(error) => {
                    warn!(?error, "MS/TP core refused its configuration");
                    return;
                }
            };
        let mut mac = Mac {
            echo: serial.echo(),
            serial,
            clock,
            npdus,
            counts,
            octet_time: Duration::from_micros(config.octet_time().as_micros()),
            read_buf: vec![0; READ_CHUNK],
            tx_frame: Vec::with_capacity(MAX_STANDARD_FRAME_LENGTH),
            outgoing: None,
            reply: None,
        };
        mac.run(&mut core).await;
    }
}

/// Refuses sends once the task is gone, however it ends.
struct CloseOnExit(SharedQueues);

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// The serial port failed a read; the task ends.
struct ReadFailed;

/// What a frame being sent counts as in the diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SentKind {
    DataExpectingReply,
    DirectAnswer,
    QueuedData,
    ReplyPostponed,
    Other,
}

impl SentKind {
    /// Classify an encoded frame; `answering` says the core is answering a
    /// request, so a data frame is that answer.
    fn of(frame: &[u8], answering: bool) -> Self {
        match frame.get(2).copied().map(FrameType::from_raw) {
            Some(FrameType::BACnetDataExpectingReply) => Self::DataExpectingReply,
            Some(FrameType::BACnetDataNotExpectingReply) if answering => Self::DirectAnswer,
            Some(FrameType::BACnetDataNotExpectingReply) => Self::QueuedData,
            Some(FrameType::ReplyPostponed) => Self::ReplyPostponed,
            _ => Self::Other,
        }
    }
}

/// A frame the core handed over that has not started.
struct Outgoing {
    id: TransmitId,
    kind: SentKind,
}

/// A request delivered upward whose answer may still come back.
struct PendingReply {
    id: RequestId,
    requester: u8,
    rx: oneshot::Receiver<Bytes>,
}

/// An action copied out of the core, so its borrow ends.
enum Polled {
    Idle,
    Receive,
    Transmit(TransmitId, SentKind),
    Deliver {
        source: u8,
        destination: u8,
        npdu: Bytes,
        request: Option<RequestId>,
    },
    Frame(FrameType, u8),
}

/// The adapter's state besides the core.
struct Mac<S> {
    serial: S,
    echo: Echo,
    clock: CoreClock,
    npdus: mpsc::Sender<ReceivedNpdu>,
    counts: Arc<Counters>,
    octet_time: Duration,
    read_buf: Vec<u8>,
    /// The octets of the frame being sent.
    tx_frame: Vec<u8>,
    outgoing: Option<Outgoing>,
    reply: Option<PendingReply>,
}

impl<S: SerialPort> Mac<S> {
    async fn run(&mut self, core: &mut Core<'_>) {
        loop {
            self.settle(core);
            if self.outgoing.is_some() {
                if self.transmit(core).await.is_err() {
                    return;
                }
                continue;
            }
            let deadline = core.next_deadline().map(|at| self.clock.instant(at));
            let wake = deadline
                .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3_600));
            // Input before a deadline at the same moment: the core judges
            // the deadline against the activity it has been given.
            tokio::select! {
                biased;
                decision = wait_reply(&mut self.reply), if self.reply.is_some() => {
                    self.answer(core, decision.ok());
                }
                read = self.serial.read(&mut self.read_buf) => match read {
                    Ok(0) => {}
                    Ok(count) => {
                        let now = self.clock.now();
                        self.receive(core, 0..count, now);
                    }
                    Err(error) => {
                        self.read_failed(&error);
                        return;
                    }
                },
                () = tokio::time::sleep_until(wake), if deadline.is_some() => self.deadline(core),
            }
        }
    }

    /// Carry out the core's actions, release a finished request's reply
    /// channel, and publish the counters.
    fn settle(&mut self, core: &mut Core<'_>) {
        self.drain_actions(core);
        let finished = self
            .reply
            .as_ref()
            .is_some_and(|reply| core.snapshot().request_id != Some(reply.id));
        if finished {
            self.release_reply(core);
        }
        self.counts.publish(core.counters());
    }

    /// Poll the core until it has nothing left.
    fn drain_actions(&mut self, core: &mut Core<'_>) {
        loop {
            match self.poll(core) {
                Polled::Idle => return,
                Polled::Receive => {
                    // Back to receive before the start: the core took the
                    // frame back, so it is not sent.
                    if self.outgoing.take().is_some() {
                        debug!("MS/TP core took back a frame before it started");
                    }
                }
                Polled::Transmit(id, kind) => self.outgoing = Some(Outgoing { id, kind }),
                Polled::Deliver {
                    source,
                    destination,
                    npdu,
                    request,
                } => self.deliver(core, source, destination, npdu, request),
                Polled::Frame(frame_type, source) => {
                    debug!(
                        ?frame_type,
                        source, "MS/TP frame without an NPDU not passed up"
                    );
                }
            }
        }
    }

    fn poll(&mut self, core: &mut Core<'_>) -> Polled {
        loop {
            let answering = core.state() == MasterState::AnswerDataRequest;
            return match core.poll_action() {
                Action::Idle => Polled::Idle,
                Action::SetDirection(Direction::Receive) => Polled::Receive,
                // Every SerialPort switches its transceiver as it writes,
                // and writes only once the start is confirmed.
                Action::SetDirection(Direction::Transmit) => continue,
                Action::Transmit { id, frame } => {
                    self.tx_frame.clear();
                    self.tx_frame.extend_from_slice(frame);
                    Polled::Transmit(id, SentKind::of(frame, answering))
                }
                Action::DeliverNpdu {
                    source,
                    destination,
                    npdu,
                    request_id,
                } => Polled::Deliver {
                    source,
                    destination,
                    npdu: Bytes::copy_from_slice(npdu),
                    request: request_id,
                },
                Action::DeliverFrame {
                    frame_type, source, ..
                } => Polled::Frame(frame_type, source),
            };
        }
    }

    /// Hand an NPDU up; a request gets a one-use reply channel.
    fn deliver(
        &mut self,
        core: &mut Core<'_>,
        source: u8,
        destination: u8,
        npdu: Bytes,
        request: Option<RequestId>,
    ) {
        let reply_tx = request.map(|id| {
            // The core holds one request at a time, so an older one is over.
            if self.reply.is_some() {
                self.release_reply(core);
            }
            let (tx, rx) = oneshot::channel();
            self.reply = Some(PendingReply {
                id,
                requester: source,
                rx,
            });
            tx
        });
        let result = self.npdus.try_send(ReceivedNpdu {
            direct_response: None,
            npdu,
            source_mac: MacAddr::from_slice(&[source]),
            link_layer_group: destination == BROADCAST_MAC,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx,
        });
        // A request that could not go up drops its reply sender, which the
        // next turn of the loop takes for Reply Postponed.
        self.counts.delivery(&result);
    }

    /// Close the reply channel of a request the core has finished. An
    /// answer already in it was too late for the core: queue it.
    fn release_reply(&mut self, core: &mut Core<'_>) {
        let Some(mut reply) = self.reply.take() else {
            return;
        };
        reply.rx.close();
        if let Ok(answer) = reply.rx.try_recv() {
            self.enqueue_late_answer(core, reply.requester, &answer);
        }
    }

    /// The application answered a request, or dropped its reply channel.
    fn answer(&mut self, core: &mut Core<'_>, answer: Option<Bytes>) {
        let Some(reply) = self.reply.take() else {
            return;
        };
        let now = self.clock.now();
        let Some(answer) = answer else {
            if let Err(error) = core.complete_request(reply.id, ReplyDecision::Postponed, now) {
                debug!(?error, "MS/TP request already finished");
            }
            return;
        };
        match core.complete_request(reply.id, ReplyDecision::Data(&answer), now) {
            Ok(()) => {}
            Err(CoreError::ReplyOversize { length, maximum }) => {
                increment(&self.counts.outbound_oversize);
                warn!("MS/TP application reply length {length} exceeds standard-frame maximum {maximum}");
                // No answer can be sent, so the request is postponed.
                let _ = core.complete_request(reply.id, ReplyDecision::Postponed, now);
            }
            // Past the decision deadline, or the request is gone: the core
            // sends or has sent Reply Postponed, and the answer follows with
            // the token.
            Err(_) => self.enqueue_late_answer(core, reply.requester, &answer),
        }
    }

    fn enqueue_late_answer(&mut self, core: &mut Core<'_>, requester: u8, answer: &[u8]) {
        let priority = npdu_priority(answer);
        match core.enqueue_npdu(requester, answer, priority, self.clock.now()) {
            Ok(()) => {}
            Err(QueueRejectReason::Oversize) => increment(&self.counts.outbound_oversize),
            Err(QueueRejectReason::Full) => increment(&self.counts.outbound_queue_full),
            Err(reason) => debug!(?reason, "MS/TP late answer not queued"),
        }
    }

    /// Feed the octets `range` of the read buffer to the core, all stamped
    /// `now`, carrying out the actions after each.
    fn receive(
        &mut self,
        core: &mut Core<'_>,
        range: Range<usize>,
        now: bacnet_mstp_core::Instant,
    ) {
        for index in range {
            let octet = self.read_buf[index];
            if let Err(error) = core.on_octet(octet, now) {
                debug!(?error, "MS/TP core refused an octet");
            }
            self.drain_actions(core);
        }
    }

    fn deadline(&mut self, core: &mut Core<'_>) {
        let before = core.state();
        let invalid = (before == MasterState::WaitForReply).then(|| core.counters().parser_errors);
        if let Err(error) = core.on_deadline(self.clock.now()) {
            debug!(?error, "MS/TP core refused a deadline");
        }
        // Leaving WAIT_FOR_REPLY at a deadline without an invalid frame (a
        // timed-out partial one) is the reply timeout.
        if let Some(invalid) = invalid {
            if core.state() != MasterState::WaitForReply && core.counters().parser_errors == invalid
            {
                increment(&self.counts.wait_for_reply_timeouts);
            }
        }
    }

    fn read_failed(&self, error: &Error) {
        increment(&self.counts.serial_read_errors);
        warn!("MS/TP serial read error: {error}");
    }

    /// Send the frame the core handed over (see the module docs).
    async fn transmit(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        self.drain_input(core).await?;
        let Some(outgoing) = self.outgoing.take() else {
            return Ok(());
        };
        let started = tokio::time::Instant::now();
        if let Err(error) = core.on_transmit_started(outgoing.id, self.clock.at(started)) {
            debug!(?error, "MS/TP transmit start refused");
            return Ok(());
        }
        let octets = u32::try_from(self.tx_frame.len()).unwrap_or(u32::MAX);
        let wire_end = started + self.octet_time * octets;
        let sent = match self.serial.write(&self.tx_frame).await {
            Ok(()) => match self.serial.drain().await {
                Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::Unsupported => {
                    Ok(())
                }
                drained => drained,
            },
            Err(error) => {
                // The failed write may have queued part of the frame.
                let _ = self.serial.drain().await;
                Err(error)
            }
        };
        if let Err(error) = sent {
            increment(&self.counts.serial_write_errors);
            warn!("MS/TP write error: {error}");
            let now = self.clock.now();
            if let Err(error) = core.on_transmit_aborted(outgoing.id, TransmitFailure::Io, now) {
                debug!(?error, "MS/TP abort refused");
            }
            return Ok(());
        }
        // The frame ended when the drain returned, or at its wire time if
        // that is later. The timer may wake a little after that; the core is
        // told the end itself.
        let ended = wire_end.max(tokio::time::Instant::now());
        if tokio::time::Instant::now() < ended {
            tokio::time::sleep_until(ended).await;
        }
        let after_echo = match self.echo {
            Echo::None => None,
            Echo::Full => self.take_echo(ended).await?,
        };
        match core.on_transmit_complete(outgoing.id, self.clock.at(ended)) {
            Ok(()) => self.count_sent(outgoing.kind),
            Err(error) => debug!(?error, "MS/TP completion refused"),
        }
        self.drain_actions(core);
        if let Some((range, read_at)) = after_echo {
            self.receive(core, range, read_at);
        }
        Ok(())
    }

    /// Read without waiting and feed the core what is already there, until
    /// nothing is or the core takes the frame back.
    async fn drain_input(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        for _ in 0..DRAIN_READS {
            let Some(read) = poll_once(self.serial.read(&mut self.read_buf)).await else {
                return Ok(());
            };
            match read {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    let now = self.clock.now();
                    self.receive(core, 0..count, now);
                }
                Err(error) => {
                    self.read_failed(&error);
                    return Err(ReadFailed);
                }
            }
            if self.outgoing.is_none() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Read and drop the echo of the frame just sent: as many octets as it
    /// has, arriving by [`ECHO_GRACE`] after `ended`. Return octets read
    /// after the echo, with the time of their read.
    async fn take_echo(
        &mut self,
        ended: tokio::time::Instant,
    ) -> Result<Option<(Range<usize>, bacnet_mstp_core::Instant)>, ReadFailed> {
        let grace_end = ended + ECHO_GRACE;
        let mut matched = 0;
        let mut mismatch = false;
        let mut kept = None;
        while matched < self.tx_frame.len() {
            let read = tokio::select! {
                biased;
                read = self.serial.read(&mut self.read_buf) => read,
                () = tokio::time::sleep_until(grace_end) => {
                    increment(&self.counts.echo_missing);
                    break;
                }
            };
            let count = match read {
                Ok(count) => count,
                Err(error) => {
                    self.read_failed(&error);
                    return Err(ReadFailed);
                }
            };
            let echo = count.min(self.tx_frame.len() - matched);
            mismatch |= self.read_buf[..echo] != self.tx_frame[matched..matched + echo];
            matched += echo;
            if echo < count {
                kept = Some((echo..count, self.clock.now()));
                break;
            }
        }
        if mismatch {
            increment(&self.counts.echo_mismatches);
        }
        Ok(kept)
    }

    fn count_sent(&self, kind: SentKind) {
        let counter = match kind {
            SentKind::DataExpectingReply => &self.counts.der_tx,
            SentKind::DirectAnswer => &self.counts.dner_tx_direct,
            SentKind::QueuedData => &self.counts.dner_tx_queued,
            SentKind::ReplyPostponed => &self.counts.reply_postponed_tx,
            SentKind::Other => return,
        };
        increment(counter);
    }
}

/// Wait for the application's answer to the pending request.
async fn wait_reply(reply: &mut Option<PendingReply>) -> Result<Bytes, oneshot::error::RecvError> {
    match reply {
        Some(reply) => (&mut reply.rx).await,
        None => std::future::pending().await,
    }
}

/// Poll `future` once: its output if it is ready now, otherwise `None`, and
/// the future is dropped.
async fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        Poll::Ready(match future.as_mut().poll(cx) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_frames_are_classified_by_type_and_answering() {
        let frame = |kind: u8| [0x55, 0xFF, kind, 1, 2, 0, 0, 0];
        assert_eq!(SentKind::of(&frame(5), false), SentKind::DataExpectingReply);
        assert_eq!(SentKind::of(&frame(6), true), SentKind::DirectAnswer);
        assert_eq!(SentKind::of(&frame(6), false), SentKind::QueuedData);
        assert_eq!(SentKind::of(&frame(7), true), SentKind::ReplyPostponed);
        assert_eq!(SentKind::of(&frame(0), false), SentKind::Other);
        assert_eq!(SentKind::of(&[0x55], false), SentKind::Other);
    }

    #[tokio::test]
    async fn poll_once_returns_only_what_is_ready() {
        assert_eq!(poll_once(async { 7 }).await, Some(7));
        assert_eq!(poll_once(std::future::pending::<u8>()).await, None);
    }
}
