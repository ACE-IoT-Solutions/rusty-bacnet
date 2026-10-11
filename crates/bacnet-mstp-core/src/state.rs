//! Clause 9.5.6 master state machine.

use bacnet_mstp_codec::{
    DecodeEvent, Frame, FrameError, FrameHeader, FrameType, StreamDecoder, StreamPhase,
    BROADCAST_MAC, HEADER_LENGTH, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

use crate::actions::{
    Action, ActionEnqueueError, ActionQueue, Direction, RequestId, TransmitFailure, TransmitId,
};
use crate::clock::{Duration, Instant};
use crate::config::{ConfigError, MstpCoreConfig, MIN_REPLY_TIMEOUT};
use crate::counters::{MstpCounters, QueueCounters};
use crate::queue::{NetworkPriority, QueueKind, QueueRejectReason, QueueStorage};

mod receive;
mod reply;
mod token;
mod transmission;

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
    StaleTransmit,
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

/// When a reply to a received request may still go out.
#[derive(Debug, Clone, Copy)]
struct ReplyWindow {
    /// The end of T_reply_delay. A reply that starts later is counted late.
    send_by: Instant,
    /// The requester's shortest possible reply timeout. From here on it may
    /// be transmitting again, so nothing is sent.
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

#[derive(Debug, Clone, Copy)]
struct ActiveTransmit {
    id: TransmitId,
    asserted_at: Instant,
    started: bool,
    post: PostTransmit,
    reply: Option<ReplyWindow>,
}

#[derive(Debug, Clone, Copy)]
struct PendingSend {
    length: usize,
    frame: [u8; MAX_STANDARD_FRAME_LENGTH],
    post: PostTransmit,
    earliest: Instant,
    reply: Option<ReplyWindow>,
}

#[derive(Debug, Clone, Copy)]
struct OwnedFrame {
    header: FrameHeader,
    length: usize,
    data: [u8; MAX_STANDARD_MPDU_DATA],
}

/// Synchronous, allocation-free MS/TP master core.
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
    pub fn on_octet(&mut self, byte: u8, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.active_transmit.is_some() {
            return Ok(()); // The platform should suppress local echo.
        }
        let timed_out_candidate = self.abort_partial_if_late(now);
        self.note_bus_activity(now);
        self.cancel_pending_send_on_activity(now);
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
        self.note_bus_activity(now);
        self.cancel_pending_send_on_activity(now);
        self.process_owned_frame(Self::copy_frame(frame), now)
    }

    /// Report host-observed bus octets before chunk-tolerant frame assembly.
    ///
    /// Host serial APIs do not preserve reliable per-octet timestamps. This
    /// updates silence and token-use accounting without feeding the exact
    /// inter-octet parser used by [`Self::on_octet`].
    pub fn on_host_activity(&mut self, octets: usize, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.active_transmit.is_some() || octets == 0 {
            return Ok(());
        }
        self.last_activity = now;
        self.event_count = self
            .event_count
            .saturating_add(u32::try_from(octets).unwrap_or(u32::MAX));
        self.cancel_pending_send_on_activity(now);
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
                self.protocol_deadline = Some(now + self.config.t_usage_timeout)
            }
            MasterState::WaitForReply => {
                self.protocol_deadline = Some(now + self.config.t_reply_timeout)
            }
            MasterState::UseToken | MasterState::DoneWithToken | MasterState::AnswerDataRequest => {
            }
        }
        Ok(())
    }

    /// Report a UART receive failure as bus activity and abort partial input.
    pub fn on_receive_error(
        &mut self,
        _error: ReceiveError,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.observe_time(now)?;
        self.note_bus_activity(now);
        self.cancel_pending_send_on_activity(now);
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

    /// Confirm that the platform is about to start the matching UART transfer.
    ///
    /// Call it after polling [`Action::Transmit`] and before enabling the
    /// line driver, and send only if it returns `Ok`. After any error for a
    /// transmit, or once it is aborted, send nothing more for its id.
    ///
    /// A reply that starts after T_reply_delay still goes out and is counted
    /// in [`MstpCounters::late_replies`]. One that would start once the
    /// requester may have stopped waiting is refused with
    /// [`CoreError::RequestExpired`] and counted as an expired request.
    pub fn on_transmit_started(&mut self, id: TransmitId, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        if tx.started {
            return Err(CoreError::TransmitAlreadyStarted);
        }
        if let Some(reply) = tx.reply {
            if now >= reply.give_up {
                self.actions.purge_transmit(id);
                self.actions
                    .enqueue_direction(Direction::Receive)
                    .map_err(|_| CoreError::ActionCapacity)?;
                self.active_transmit = None;
                self.counters.record_expired_request();
                self.enter_idle();
                return Err(CoreError::RequestExpired);
            }
            if now > reply.send_by {
                self.counters.record_late_reply();
            }
        }
        let tx = self.active_transmit.as_mut().expect("checked above");
        tx.started = true;
        tx.asserted_at = now;
        Ok(())
    }

    /// Report that the final stop bit for the matching generation left UART.
    ///
    /// The start must have been confirmed first. Completion without it is
    /// refused with [`CoreError::TransmitNotStarted`] and counted in
    /// [`MstpCounters::adapter_errors`]; the transmit stays active until it
    /// is started and completed, aborted, or caught by the watchdog.
    pub fn on_transmit_complete(&mut self, id: TransmitId, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        if !tx.started {
            self.counters.record_adapter_error();
            return Err(CoreError::TransmitNotStarted);
        }
        self.actions
            .enqueue_direction(Direction::Receive)
            .map_err(|_| CoreError::ActionCapacity)?;
        self.active_transmit = None;
        self.last_activity = now;
        self.counters
            .record_de_assertion(now.saturating_duration_since(tx.asserted_at));
        self.counters.record_frame_transmitted();
        match tx.post {
            PostTransmit::Idle => {
                self.set_state(MasterState::Idle);
                self.protocol_deadline = Some(now + self.config.t_no_token);
                Ok(())
            }
            PostTransmit::ContinueToken => {
                self.set_state(MasterState::UseToken);
                self.drive_token(now)
            }
            PostTransmit::WaitForReply => {
                self.set_state(MasterState::WaitForReply);
                self.protocol_deadline = Some(now + self.config.t_reply_timeout);
                Ok(())
            }
            PostTransmit::PassToken => {
                self.set_state(MasterState::PassToken);
                self.event_count = 0;
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                self.counters.record_token_rotation();
                Ok(())
            }
            PostTransmit::PollForMaster => {
                self.set_state(MasterState::PollForMaster);
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                Ok(())
            }
        }
    }

    /// Abort the matching transmit and restore logical receive direction.
    ///
    /// Its actions not yet polled are withdrawn, so the next action is the
    /// switch back to receive.
    pub fn on_transmit_aborted(
        &mut self,
        id: TransmitId,
        _reason: TransmitFailure,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.observe_time(now)?;
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        self.actions.purge_transmit(id);
        self.actions
            .enqueue_direction(Direction::Receive)
            .map_err(|_| CoreError::ActionCapacity)?;
        self.active_transmit = None;
        self.counters
            .record_de_assertion(now.saturating_duration_since(tx.asserted_at));
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(now + self.config.t_no_token);
        Ok(())
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

    fn note_bus_activity(&mut self, now: Instant) {
        self.last_activity = now;
        self.event_count = self.event_count.saturating_add(1);
        match self.state {
            MasterState::NoToken => {
                self.set_state(MasterState::Idle);
                self.protocol_deadline = Some(now + self.config.t_no_token);
            }
            MasterState::Idle => self.protocol_deadline = Some(now + self.config.t_no_token),
            MasterState::PassToken | MasterState::PollForMaster => {
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                if self.state == MasterState::PassToken
                    && self.event_count > u32::from(self.config.n_min_octets)
                {
                    self.set_state(MasterState::Idle);
                    self.protocol_deadline = Some(now + self.config.t_no_token);
                }
            }
            MasterState::WaitForReply => {
                self.protocol_deadline = Some(now + self.config.t_reply_timeout)
            }
            _ => {}
        }
    }

    /// Line activity while a frame waits out T_turnaround. A reply waits a
    /// fresh turnaround from this activity; its window still applies when it
    /// is dispatched. Any other frame is dropped, counted in
    /// [`MstpCounters::cancelled_sends`].
    fn cancel_pending_send_on_activity(&mut self, now: Instant) {
        let Some(pending) = self.pending_send.as_mut() else {
            return;
        };
        if pending.reply.is_some() {
            pending.earliest = self.last_activity + self.config.t_turnaround;
            self.protocol_deadline = Some(pending.earliest);
            return;
        }
        self.pending_send = None;
        self.counters.record_cancelled_send();
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(now + self.config.t_no_token);
    }

    /// Go idle, with the token-loss timer running from the last activity on
    /// the line (Clause 9.5.6.2).
    fn enter_idle(&mut self) {
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(self.last_activity + self.config.t_no_token);
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
