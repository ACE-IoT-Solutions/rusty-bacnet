//! Clause 9.5.6 master state machine.

use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameHeader, FrameType, StreamDecoder, StreamPhase,
    BROADCAST_MAC, HEADER_LENGTH, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

use crate::actions::{
    Action, ActionEnqueueError, ActionQueue, Direction, RequestId, TransmitFailure, TransmitId,
};
use crate::clock::{Duration, Instant};
use crate::config::{ConfigError, MstpCoreConfig, MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT};
use crate::counters::{MstpCounters, QueueCounters};
use crate::queue::{NetworkPriority, QueueKind, QueueRejectReason, QueueStorage};

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
    /// The bounded control-action ring could not accept protocol work.
    ActionCapacity,
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
    /// The reply is too large for a standard MS/TP frame.
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

#[derive(Debug, Clone, Copy)]
struct OwnedFrame {
    header: FrameHeader,
    length: usize,
    data: [u8; MAX_STANDARD_MPDU_DATA],
}

/// Synchronous, allocation-free MS/TP master core.
///
/// # Driving it
///
/// Feed it what the line does, timestamped by the caller's monotonic clock:
/// [`Self::on_octet`] or [`Self::on_receive_error`] for each received
/// character (or [`Self::on_validated_frame`] and [`Self::on_host_activity`]
/// from a host-side assembler), and [`Self::on_deadline`] at
/// [`Self::next_deadline`]. After each call, carry out the actions from
/// [`Self::poll_action`] until it returns [`Action::Idle`].
///
/// A transmit goes through three calls. Poll [`Action::Transmit`]; when the
/// driver is about to go on, call [`Self::on_transmit_started`] and drive the
/// line only if it returns `Ok`; at the final stop bit, call
/// [`Self::on_transmit_complete`]. Until the start is confirmed, keep
/// receiving and feed the core what arrives: line activity in that time
/// pushes the frame back (Clause 9.5.5.1), and the core may take the frame
/// back on its own, queueing [`Direction::Receive`] before the start; drop
/// the frame then. Octets received after the confirmed start are taken for
/// the station's own echo and ignored.
///
/// Line activity only delays a frame the core waits to send. A frame is
/// dropped, and counted, only on the evidence its sending state watches for
/// (another station using the token after a retry, or active while this one
/// generates a token) or at a time limit taken from that state. The crate's
/// `src/state/pending.rs` tables every frame that can wait, with its rules
/// and clauses.
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
pub struct MasterCore<'rx, Q: QueueStorage> {
    config: MstpCoreConfig,
    queues: Q,
    decoder: StreamDecoder<'rx>,
    actions: ActionQueue,
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

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    /// Construct a core over caller-owned receive storage.
    ///
    /// `rx_storage` is the stream decoder's buffer for [`Self::on_octet`].
    /// Size it for the largest frame this station accepts:
    /// [`MAX_STANDARD_FRAME_LENGTH`] takes any standard frame. A longer frame
    /// addressed to this station or broadcast counts as an invalid frame
    /// (Clause 9.5.4.4). Storage shorter than [`MIN_RX_STORAGE`] is refused.
    pub fn new(
        config: MstpCoreConfig,
        mut queues: Q,
        rx_storage: &'rx mut [u8],
        now: Instant,
    ) -> Result<Self, CoreError> {
        config.validate()?;
        if rx_storage.len() < MIN_RX_STORAGE {
            return Err(CoreError::ReceiveStorageTooSmall {
                length: rx_storage.len(),
                minimum: MIN_RX_STORAGE,
            });
        }
        queues.set_starvation_limit(config.starvation_limit);
        Ok(Self {
            config,
            queues,
            decoder: StreamDecoder::new(rx_storage),
            actions: ActionQueue::new(),
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
    /// core itself refused or dropped as too long for a standard frame.
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
    /// An NPDU longer than a standard frame carries (501 octets) is refused
    /// with [`QueueRejectReason::Oversize`] and counted in its queue's
    /// `drops_oversize`, whatever the queue storage could hold.
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
        if npdu.len() > MAX_STANDARD_MPDU_DATA {
            self.record_oversize_npdu(QueueKind::from_destination(destination));
            return Err(QueueRejectReason::Oversize);
        }
        let expires = now.saturating_add(self.config.queue_max_age).as_micros();
        self.queues
            .enqueue_npdu(destination, npdu, priority, now.as_micros(), Some(expires))
    }

    /// Feed one wire-timestamped octet through the incremental codec.
    ///
    /// `now` is when the octet arrived. When it completes a frame, every
    /// reply time for that frame is measured from it.
    ///
    /// A candidate the codec drops counts as an invalid frame only as
    /// Clause 9.5.4 would count it for this station; see
    /// [`DecodeEvent::Malformed`].
    ///
    /// Once a transmit's start is confirmed, octets are taken for the
    /// station's own echo and ignored until it completes. Before that they
    /// are line activity like any other (see [`Self::on_transmit_started`]).
    pub fn on_octet(&mut self, byte: u8, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.transmitting() {
            return Ok(());
        }
        let timed_out_candidate = self.abort_partial_if_late(now);
        self.note_bus_activity(now, 1)?;
        if timed_out_candidate {
            self.invalid_frame_event(now)?;
        }
        self.last_octet = Some(now);
        let result = self.decode_octet(byte, now);
        self.parser_partial = self.decoder.phase() != StreamPhase::Sync;
        result
    }

    /// Feed a frame already validated by a host-side chunk-tolerant assembler.
    ///
    /// `now` should be the adapter's best estimate of when the frame's final
    /// octet arrived: a reply to the frame is timed from it, and a late
    /// estimate leaves the requester less time than the core assumes.
    ///
    /// A frame the codec would not have decoded is refused with
    /// [`CoreError::Frame`] and changes nothing, not even the core's clock:
    /// more data than a standard frame carries (501 octets), source 255, or
    /// a COBS-encoded type.
    ///
    /// How this host path treats the station's own echo, and frames whose
    /// source is this station, is still open: #1617 settles both.
    pub fn on_validated_frame(&mut self, frame: Frame<'_>, now: Instant) -> Result<(), CoreError> {
        let header = frame.header;
        if frame.data.len() > MAX_STANDARD_MPDU_DATA {
            return Err(CoreError::Frame(FrameError::Oversize {
                length: frame.data.len(),
                maximum: MAX_STANDARD_MPDU_DATA,
            }));
        }
        if header.source == BROADCAST_MAC {
            return Err(CoreError::Frame(FrameError::BroadcastSource));
        }
        if header.frame_type.is_cobs() {
            return Err(CoreError::Frame(FrameError::UnsupportedCobsFrame {
                raw_type: header.frame_type.to_raw(),
            }));
        }
        self.observe_time(now)?;
        self.note_bus_activity(now, 1)?;
        self.process_owned_frame(Self::copy_frame(frame), now)
    }

    /// Report host-observed bus octets before chunk-tolerant frame assembly.
    ///
    /// Host serial APIs do not preserve reliable per-octet timestamps. This
    /// updates silence and token-use accounting without feeding the exact
    /// inter-octet parser used by [`Self::on_octet`].
    ///
    /// Like [`Self::on_octet`], it is ignored as echo only once a transmit's
    /// start is confirmed.
    pub fn on_host_activity(&mut self, octets: usize, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.transmitting() || octets == 0 {
            return Ok(());
        }
        self.note_bus_activity(now, u32::try_from(octets).unwrap_or(u32::MAX))
    }

    /// Report a UART receive failure as bus activity and abort partial input.
    ///
    /// It is taken for the station's own echo exactly when
    /// [`Self::on_octet`] would be: once a transmit's start is confirmed.
    pub fn on_receive_error(
        &mut self,
        _error: ReceiveError,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.transmitting() {
            return Ok(());
        }
        self.note_bus_activity(now, 1)?;
        let invalid_candidate = matches!(
            self.decoder.phase(),
            StreamPhase::Header | StreamPhase::Data
        );
        self.decoder.reset();
        self.parser_partial = false;
        if invalid_candidate {
            self.counters.record_parser_error();
            self.invalid_frame_event(now)?;
        }
        Ok(())
    }

    /// Apply exact wire silence; a partial frame aborts only when silence is
    /// strictly greater than T_frame_abort.
    pub fn on_silence(&mut self, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.abort_partial_if_late(now) {
            self.invalid_frame_event(now)?;
        }
        Ok(())
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
    pub fn poll_action(&mut self) -> Action<'_> {
        self.actions.poll_action().unwrap_or(Action::Idle)
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

    /// Count an NPDU refused or dropped as too long for a standard frame.
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

fn next_addr(current: u8, max_master: u8) -> u8 {
    if current >= max_master {
        0
    } else {
        current + 1
    }
}

fn minimum(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(core::cmp::min(left, right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn scale(duration: Duration, count: u64) -> Duration {
    Duration::from_micros(duration.as_micros().saturating_mul(count))
}
