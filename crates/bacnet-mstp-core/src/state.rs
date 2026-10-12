//! Clause 9.5.6 master state machine.

use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameType, StreamDecoder, StreamPhase, BROADCAST_MAC,
    HEADER_LENGTH, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA,
    PREAMBLE,
};
use bacnet_time::{Duration, Instant};

use crate::actions::{
    carries_npdu, Action, ActionBuffers, ActionQueue, RequestId, TransmitFailure, TransmitId,
};
use crate::config::{
    ConfigError, CoreBuffer, MstpCoreConfig, MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT,
};
use crate::counters::{MstpCounters, QueueCounters};
use crate::queue::{NetworkPriority, QueueKind, QueueRejectReason, QueueStorage};

mod input;
mod pending;
mod receive;
mod reply;
mod token;
mod transmission;

use pending::{Origin, PendingSend};

/// The smallest receive storage [`MasterCore::new`] accepts: a preamble and
/// a header, the size of the shortest frame.
pub const MIN_RX_STORAGE: usize = PREAMBLE.len() + HEADER_LENGTH;

/// A deadline serviced at least this long after it fell due counts in
/// [`MstpCounters::deadline_misses`]. Smaller lateness is normal for an
/// adapter driven by a timer tick.
pub const LATE_DEADLINE: Duration = Duration::from_millis(1);

/// Token-passing master state names from Clause 9.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterState {
    /// Waiting for a frame or token-loss timeout.
    Idle,
    /// Waiting for this station's token-generation slot.
    NoToken,
    /// Holding the token and selecting information frames.
    UseToken,
    /// Selecting token-pass or maintenance work.
    DoneWithToken,
    /// The token went to the next station; watching the line for traffic that
    /// shows it was taken up.
    PassToken,
    /// Waiting for a reply to a data-expecting-reply frame.
    WaitForReply,
    /// Waiting for a ReplyToPollForMaster frame.
    PollForMaster,
    /// Waiting for the application to answer a received request.
    AnswerDataRequest,
}

/// UART receive failures that count as bus activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    /// Character framing failure.
    Framing,
    /// UART overrun.
    Overrun,
    /// Character parity failure.
    Parity,
    /// Break condition.
    Break,
}

/// Application decision for a request-bearing delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyDecision<'a> {
    /// Send a BACnetDataNotExpectingReply response immediately.
    Data(&'a [u8]),
    /// Tell the requester that the response will be sent later with a token.
    Postponed,
    /// The application abandoned or cancelled the bound request.
    Cancelled,
    /// Abandon the bound request without emitting a wire response.
    Abandoned,
}

/// A core event was rejected without performing the requested operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreError {
    /// Invalid core configuration.
    Config(ConfigError),
    /// The caller supplied a timestamp older than the previous event.
    ClockRegression {
        /// Timestamp accepted for the preceding event.
        previous: Instant,
        /// Regressing timestamp supplied by the caller.
        supplied: Instant,
    },
    /// A frame could not be encoded, or a frame handed to
    /// [`MasterCore::on_validated_frame`] is not a standard frame.
    Frame(FrameError),
    /// No application request is pending.
    NoPendingRequest,
    /// The request identifier is stale or belongs to another transaction.
    StaleRequest,
    /// Too late for this reply. Either a data reply came after the decision
    /// deadline (the core still owns the request and sends Reply Postponed),
    /// or the requester may have stopped waiting, so the request was released
    /// unanswered.
    RequestExpired,
    /// The reply is longer than this station sends: 501 octets, or 1497
    /// with [`MstpCoreConfig::extended_frames`] on.
    ReplyOversize {
        /// Supplied reply length.
        length: usize,
        /// Maximum supported reply length.
        maximum: usize,
    },
    /// The transmit identifier is stale or does not match the active transfer.
    /// A frame the core took back before its start was confirmed (see
    /// [`MasterCore::on_transmit_started`]) is stale too.
    StaleTransmit,
    /// The line was active within T_turnaround of the reported start, so the
    /// frame was taken back to wait for a quiet line again (Clause 9.5.5.1).
    /// Send nothing for this id; the frame comes back as a new
    /// [`Action::Transmit`] unless its waiting rules drop it.
    TransmitDeferred,
    /// A transmit-start event was reported more than once.
    TransmitAlreadyStarted,
    /// A start was reported for a transmit whose [`Action::Transmit`] has
    /// not been polled yet, so the adapter cannot have its octets. An
    /// adapter bug; unlike the other errors it leaves the id live: poll the
    /// frame, then start it.
    TransmitNotPolled,
    /// Completion was reported for a transmit whose start was never confirmed.
    TransmitNotStarted,
    /// The receive storage cannot hold even the shortest frame.
    ReceiveStorageTooSmall {
        /// Supplied storage length.
        length: usize,
        /// [`MIN_RX_STORAGE`].
        minimum: usize,
    },
}

impl From<ConfigError> for CoreError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<FrameError> for CoreError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

/// Copyable state for diagnostics without exposing mutable internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasterSnapshot {
    /// Current Clause 9.5.6 state.
    pub state: MasterState,
    /// Known successor station.
    pub next_station: u8,
    /// Station currently being polled.
    pub poll_station: u8,
    /// Information frames sent during the current token use.
    pub frame_count: u8,
    /// Maintenance token count.
    pub token_count: u8,
    /// Whether this station currently believes it is the sole master.
    pub sole_master: bool,
    /// Active transmit generation.
    pub transmit_id: Option<TransmitId>,
    /// Active application request generation.
    pub request_id: Option<RequestId>,
}

#[derive(Debug, Clone, Copy)]
struct PendingRequest {
    id: RequestId,
    source: u8,
    /// When the core stops waiting for the application and sends Reply
    /// Postponed: T_reply_delay less the transmit margin.
    decide_by: Instant,
    window: ReplyWindow,
}

/// When an answer to a received frame may still go out.
#[derive(Debug, Clone, Copy)]
struct ReplyWindow {
    /// The end of T_reply_delay, or of T_usage_delay for Reply To Poll For
    /// Master. An answer that starts later is counted late.
    send_by: Instant,
    /// When the asker may stop waiting, at its shortest timeout. From here
    /// on it may be transmitting again, so nothing is sent.
    give_up: Instant,
}

#[derive(Debug, Clone, Copy)]
enum PostTransmit {
    Idle,
    ContinueToken,
    WaitForReply,
    PassToken,
    PollForMaster,
}

/// A frame handed to the adapter, from the hand-over until it completes or
/// is aborted or taken back.
#[derive(Debug, Clone, Copy)]
struct ActiveTransmit {
    id: TransmitId,
    asserted_at: Instant,
    /// The adapter confirmed the start; until then the frame is also still
    /// the waiting [`PendingSend`].
    started: bool,
    post: PostTransmit,
}

/// Synchronous, allocation-free MS/TP master core.
///
/// # Driving it
///
/// Feed it what the line does, timestamped by the caller's monotonic clock:
/// [`Self::on_octet`] or [`Self::on_receive_error`] for each received
/// character, or from a host-side assembler [`Self::on_host_activity`] for
/// raw octets and [`Self::on_validated_frame`] or [`Self::on_invalid_frame`]
/// for each frame it completes; and [`Self::on_deadline`] at
/// [`Self::next_deadline`].
///
/// After each call, carry out the actions from [`Self::poll_action`] until it
/// returns [`Action::Idle`], and before the next transmit event at the
/// latest. Control actions never run out of room: at most a switch to
/// receive and one transmit wait at a time. Received data waits in the
/// delivery slots of [`ActionBuffers`], and data that finds them all full is
/// dropped and counted in [`MstpCounters::delivery_drops`].
///
/// A transmit goes through three calls. Poll [`Action::Transmit`]; when the
/// driver is about to go on, call [`Self::on_transmit_started`] and drive the
/// line only if it returns `Ok`; at the final stop bit, call
/// [`Self::on_transmit_complete`]. Until the start is confirmed, keep
/// receiving and feed the core what arrives, draining the receive FIFO or
/// DMA buffer right before the start call (octets stamped with the drain
/// time are safe: a later stamp can only defer the start). Line activity in
/// that time pushes the frame back (Clause 9.5.5.1), and the core may take
/// the frame back on its own, queueing
/// [`Direction::Receive`](crate::Direction::Receive) before the start; drop
/// the frame then. Input after the confirmed start is taken for the
/// station's own echo and ignored, whichever call reports it.
///
/// Line activity only delays a frame the core waits to send. A frame is
/// dropped, and counted, only on the evidence its sending state watches for
/// (another station using the token after a retry, or active while this one
/// generates a token) or at a time limit taken from that state. The crate's
/// `src/state/pending.rs` tables every frame that can wait, with its rules
/// and clauses.
///
/// # Storage
///
/// The core borrows its two large buffers, the receive storage and the
/// [`ActionBuffers`], for its lifetime `'rx`, and owns its queues `Q`, which
/// may be a `&mut` borrow of queues kept elsewhere. Everything it holds by
/// value is small, so [`Self::new`] can return it on a microcontroller's
/// stack; the crate docs show the pattern.
///
/// `NPDU` is the size of the [`ActionBuffers`] delivery slots, which the
/// core takes from them, and of the scratch it copies a queued NPDU into on
/// the stack to send it. It defaults to a standard frame's 501 octets, so a
/// core without extended frames costs what it always did; one with
/// [`MstpCoreConfig::extended_frames`] on needs
/// [`MAX_EXTENDED_MPDU_DATA`]
/// (see [`ExtendedActionBuffers`](crate::ExtendedActionBuffers)).
///
/// # Late token generation (a local policy)
///
/// Clause 9.5.6.7 lets a station generate a token only while the line's
/// silence lies inside the station's own slot, and gives it no way on once
/// that slot has gone by in silence: read literally, a sole master whose
/// NO_TOKEN deadline was serviced late would wait there for ever. The core
/// opens the station's window again every T_slot × (Max_Master + 1) of
/// continued silence instead, which keeps every station's turn in the same
/// order, and counts each token generated that way in
/// [`MstpCounters::missed_generation_slots`]. It departs from the clause only
/// when the deadline is serviced after the slot has closed; on time, the
/// first slot is used.
pub struct MasterCore<'rx, Q: QueueStorage, const NPDU: usize = MAX_STANDARD_MPDU_DATA> {
    config: MstpCoreConfig,
    queues: Q,
    decoder: StreamDecoder<'rx>,
    actions: ActionQueue<'rx, NPDU>,
    state: MasterState,
    next_station: u8,
    poll_station: u8,
    token_count: u8,
    frame_count: u8,
    retry_token_count: u8,
    event_count: u32,
    sole_master: bool,
    /// A Token for this station arrived while it was answering a request;
    /// it is taken up on the way back to IDLE.
    pending_token: bool,
    pending_request: Option<PendingRequest>,
    next_request_generation: u32,
    active_transmit: Option<ActiveTransmit>,
    pending_send: Option<PendingSend>,
    next_transmit_generation: u32,
    protocol_deadline: Option<Instant>,
    last_activity: Instant,
    last_octet: Option<Instant>,
    parser_partial: bool,
    last_now: Instant,
    consecutive_broadcast: u8,
    counters: MstpCounters,
}

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Construct a core over caller-owned storage.
    ///
    /// `rx_storage` is the stream decoder's buffer for [`Self::on_octet`].
    /// Size it for the largest frame this station accepts:
    /// [`MAX_STANDARD_FRAME_LENGTH`](bacnet_mstp_codec::MAX_STANDARD_FRAME_LENGTH)
    /// takes any standard frame, and
    /// [`MAX_EXTENDED_FRAME_LENGTH`] any extended frame
    /// that carries a BACnet NPDU. A longer frame addressed to this station
    /// or broadcast counts as an invalid frame (Clause 9.5.4.4), at its
    /// header; one for another station is skipped. Without
    /// [`MstpCoreConfig::extended_frames`] an extended frame the storage
    /// holds counts the same way at its last octet. Storage shorter than
    /// [`MIN_RX_STORAGE`] is refused.
    ///
    /// `action_buffers` holds the frame this station sends and the received
    /// data waiting to be polled; its `DELIVERIES` sets how much may wait,
    /// and its `NPDU` how long each may be (see [`ActionBuffers`]). Whatever
    /// either buffer held is overwritten before it is read.
    ///
    /// With [`MstpCoreConfig::extended_frames`] on, every buffer must hold
    /// what an extended frame needs: the receive storage and the action
    /// buffers' frame [`MAX_EXTENDED_FRAME_LENGTH`] octets, and a delivery
    /// slot and a queue slot ([`QueueStorage::max_npdu_length`])
    /// [`MAX_EXTENDED_MPDU_DATA`]. A smaller one is refused with
    /// [`ConfigError::ExtendedStorageTooSmall`], naming it.
    pub fn new<const DELIVERIES: usize, const FRAME: usize>(
        config: MstpCoreConfig,
        mut queues: Q,
        rx_storage: &'rx mut [u8],
        action_buffers: &'rx mut ActionBuffers<DELIVERIES, NPDU, FRAME>,
        now: Instant,
    ) -> Result<Self, CoreError> {
        config.validate()?;
        if rx_storage.len() < MIN_RX_STORAGE {
            return Err(CoreError::ReceiveStorageTooSmall {
                length: rx_storage.len(),
                minimum: MIN_RX_STORAGE,
            });
        }
        if config.extended_frames {
            for (buffer, length, minimum) in [
                (
                    CoreBuffer::ReceiveStorage,
                    rx_storage.len(),
                    MAX_EXTENDED_FRAME_LENGTH,
                ),
                (CoreBuffer::TransmitFrame, FRAME, MAX_EXTENDED_FRAME_LENGTH),
                (CoreBuffer::DeliverySlot, NPDU, MAX_EXTENDED_MPDU_DATA),
                (
                    CoreBuffer::QueueSlot,
                    queues.max_npdu_length(),
                    MAX_EXTENDED_MPDU_DATA,
                ),
            ] {
                if length < minimum {
                    return Err(CoreError::Config(ConfigError::ExtendedStorageTooSmall {
                        buffer,
                        length,
                        minimum,
                    }));
                }
            }
        }
        queues.set_starvation_limit(config.starvation_limit);
        let decoder = StreamDecoder::new(rx_storage);
        Ok(Self {
            config,
            queues,
            // A node with COBS support checks every COBS type's encoded
            // fields, the reserved ones included (Clause 9.5.4.9).
            decoder: if config.extended_frames {
                decoder.with_reserved_cobs()
            } else {
                decoder
            },
            actions: ActionQueue::new(action_buffers),
            state: MasterState::Idle,
            next_station: config.station,
            poll_station: config.station,
            token_count: config.n_poll,
            frame_count: 0,
            retry_token_count: 0,
            event_count: 0,
            sole_master: false,
            pending_token: false,
            pending_request: None,
            next_request_generation: 0,
            active_transmit: None,
            pending_send: None,
            next_transmit_generation: 0,
            protocol_deadline: Some(now + config.t_no_token),
            last_activity: now,
            last_octet: None,
            parser_partial: false,
            last_now: now,
            consecutive_broadcast: 0,
            counters: MstpCounters::default(),
        })
    }

    /// Return the current state.
    pub const fn state(&self) -> MasterState {
        self.state
    }

    /// Return the validated configuration.
    pub const fn config(&self) -> &MstpCoreConfig {
        &self.config
    }

    /// Return a copyable diagnostic snapshot.
    pub fn snapshot(&self) -> MasterSnapshot {
        MasterSnapshot {
            state: self.state,
            next_station: self.next_station,
            poll_station: self.poll_station,
            frame_count: self.frame_count,
            token_count: self.token_count,
            sole_master: self.sole_master,
            transmit_id: self.active_transmit.map(|tx| tx.id),
            request_id: self.pending_request.map(|request| request.id),
        }
    }

    /// Return the counters snapshot, including current queue statistics.
    ///
    /// The queue counters come from the queue storage, plus the NPDUs the
    /// core itself refused or dropped as too long to send
    /// ([`MstpCoreConfig::max_npdu_length`]).
    pub fn counters(&self) -> MstpCounters {
        let merge = |queue: QueueCounters, core: QueueCounters| QueueCounters {
            drops_oversize: queue.drops_oversize.saturating_add(core.drops_oversize),
            ..queue
        };
        let mut counters = self.counters;
        counters.unicast = merge(self.queues.stats(QueueKind::Unicast), self.counters.unicast);
        counters.broadcast = merge(
            self.queues.stats(QueueKind::Broadcast),
            self.counters.broadcast,
        );
        counters
    }

    /// Queue an NPDU with an absolute expiry derived from configuration.
    ///
    /// An NPDU longer than this station sends
    /// ([`MstpCoreConfig::max_npdu_length`]: 501 octets, or 1497 with
    /// extended frames on) is refused with [`QueueRejectReason::Oversize`]
    /// and counted in its queue's `drops_oversize`, whatever the queue
    /// storage could hold. With extended frames on, one of 502 octets or
    /// more goes out in an extended frame.
    pub fn enqueue_npdu(
        &mut self,
        destination: u8,
        npdu: &[u8],
        priority: NetworkPriority,
        now: Instant,
    ) -> Result<(), QueueRejectReason> {
        if now < self.last_now {
            return Err(QueueRejectReason::Expired);
        }
        if npdu.len() > self.config.max_npdu_length() {
            self.record_oversize_npdu(QueueKind::from_destination(destination));
            return Err(QueueRejectReason::Oversize);
        }
        let expires = now.saturating_add(self.config.queue_max_age);
        self.queues
            .enqueue_npdu(destination, npdu, priority, now, Some(expires))
    }

    /// Service the current absolute deadline. Early/stale callbacks are no-ops.
    pub fn on_deadline(&mut self, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.abort_partial_if_late(now) {
            self.invalid_frame_event(now)?;
        }
        let Some(deadline) = self.next_deadline() else {
            return Ok(());
        };
        if now < deadline {
            return Ok(());
        }
        if now - deadline >= LATE_DEADLINE {
            self.counters.record_deadline_miss();
        }
        if let Some(tx) = self.active_transmit {
            if now >= tx.asserted_at + self.config.transmit_watchdog {
                return self.on_transmit_aborted(tx.id, TransmitFailure::Timeout, now);
            }
            return Ok(());
        }
        if self.pending_send.is_some() {
            return self.dispatch_pending_send(now);
        }
        match self.state {
            MasterState::Idle => self.enter_no_token(now),
            MasterState::NoToken => self.generate_token_if_in_slot(now),
            MasterState::WaitForReply => {
                // Clause 9.5.6.4 ends this token use after a reply timeout;
                // queued information frames wait for the next token.
                self.frame_count = self.config.max_info_frames;
                self.set_state(MasterState::DoneWithToken);
                self.drive_token(now)
            }
            MasterState::AnswerDataRequest => self.reply_deadline(now),
            MasterState::PassToken => self.pass_token_timeout(now),
            MasterState::PollForMaster => self.poll_timeout(now),
            MasterState::UseToken | MasterState::DoneWithToken => self.drive_token(now),
        }
    }

    /// Return the next action, or [`Action::Idle`] when no action is pending.
    ///
    /// Control actions come first, then deliveries in the order they were
    /// queued. An action's octets borrow the core until the next call that
    /// needs it mutably.
    pub fn poll_action(&mut self) -> Action<'_> {
        self.actions.poll()
    }

    /// Return the earliest parser, protocol, or transmit-watchdog deadline.
    pub fn next_deadline(&self) -> Option<Instant> {
        let mut deadline = self.protocol_deadline;
        if self.parser_partial {
            if let Some(last_octet) = self.last_octet {
                let parser = last_octet + self.config.t_frame_abort + Duration::from_micros(1);
                deadline = minimum(deadline, Some(parser));
            }
        }
        if let Some(tx) = self.active_transmit {
            deadline = minimum(
                deadline,
                Some(tx.asserted_at + self.config.transmit_watchdog),
            );
        }
        deadline
    }

    fn observe_time(&mut self, now: Instant) -> Result<(), CoreError> {
        if now < self.last_now {
            return Err(CoreError::ClockRegression {
                previous: self.last_now,
                supplied: now,
            });
        }
        self.last_now = now;
        Ok(())
    }

    /// Line activity that is not this station's own echo, worth `events`
    /// toward EventCount (Clause 9.5.6). A frame of this station's that waits
    /// to go out takes it under its own rules (see the `pending` module);
    /// otherwise the current state's timers and transitions apply.
    fn note_bus_activity(&mut self, now: Instant, events: u32) -> Result<(), CoreError> {
        self.last_activity = now;
        self.event_count = self.event_count.saturating_add(events);
        if self.pending_send.is_some() {
            return self.pending_activity(now);
        }
        match self.state {
            MasterState::NoToken => {
                self.set_state(MasterState::Idle);
                self.protocol_deadline = Some(now + self.config.t_no_token);
            }
            MasterState::Idle => self.protocol_deadline = Some(now + self.config.t_no_token),
            MasterState::PassToken => {
                if self.event_count > u32::from(self.config.n_min_octets) {
                    self.set_state(MasterState::Idle);
                    self.protocol_deadline = Some(now + self.config.t_no_token);
                } else {
                    self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                }
            }
            MasterState::PollForMaster => {
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
            }
            MasterState::WaitForReply => {
                self.protocol_deadline = Some(now + self.config.t_reply_timeout);
            }
            MasterState::UseToken | MasterState::DoneWithToken | MasterState::AnswerDataRequest => {
            }
        }
        Ok(())
    }

    /// Go idle, with the token-loss timer running from the last activity on
    /// the line (Clause 9.5.6.2), then take up any Token that arrived while
    /// this station was answering a request.
    fn enter_idle(&mut self, now: Instant) -> Result<(), CoreError> {
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(self.last_activity + self.config.t_no_token);
        self.take_pending_token(now)
    }

    /// Act on a Token remembered while answering a request, as IDLE acts on
    /// a received one.
    fn take_pending_token(&mut self, now: Instant) -> Result<(), CoreError> {
        if core::mem::take(&mut self.pending_token) {
            self.use_received_token(now)
        } else {
            Ok(())
        }
    }

    /// Count an NPDU refused or dropped as too long to send.
    fn record_oversize_npdu(&mut self, kind: QueueKind) {
        let queue = match kind {
            QueueKind::Unicast => &mut self.counters.unicast,
            QueueKind::Broadcast => &mut self.counters.broadcast,
        };
        queue.drops_oversize = queue.drops_oversize.saturating_add(1);
    }

    fn set_state(&mut self, state: MasterState) {
        if self.state != state {
            self.state = state;
            self.counters.record_transition();
        }
    }
}

/// The address after `current`: (current + 1) modulo (Max_Master + 1), as
/// Clause 9.5.6 computes it.
fn next_addr(current: u8, max_master: u8) -> u8 {
    match current.cmp(&max_master) {
        core::cmp::Ordering::Less => current + 1,
        core::cmp::Ordering::Equal => 0,
        // A successor above Max_Master, which a station outside the master
        // range becomes by answering a poll, and whose failure the next
        // search starts after (FindNewSuccessor, Clause 9.5.6.6).
        core::cmp::Ordering::Greater => {
            ((u32::from(current) + 1) % (u32::from(max_master) + 1)) as u8
        }
    }
}

fn minimum(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(core::cmp::min(left, right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}
