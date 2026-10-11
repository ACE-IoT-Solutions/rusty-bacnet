//! Answering a received data-expecting-reply frame (Clause 9.5.6.9).
//!
//! The application has until T_reply_delay less the configured transmit
//! margin to decide. After that the core sends Reply Postponed itself, even
//! when its deadline callback or the platform's transmit start comes late: the
//! requester keeps waiting for at least the shortest T_reply_timeout (255 ms,
//! Clause 9.5.3), so a late answer still spares it that wait, while no answer
//! costs it the whole timeout. Only once that much time has passed does the
//! core give up on the request, counting it, because the requester may be
//! transmitting again by then.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    /// Hand a request to the application and open its reply window.
    pub(super) fn begin_request(
        &mut self,
        frame: &OwnedFrame,
        now: Instant,
    ) -> Result<(), CoreError> {
        let id = RequestId::new(self.next_request_generation);
        self.next_request_generation = self.next_request_generation.wrapping_add(1);
        let send_by = now + self.config.t_reply_delay;
        let request = PendingRequest {
            id,
            source: frame.header.source,
            decide_by: send_by - self.config.reply_transmit_margin,
            window: ReplyWindow {
                send_by,
                give_up: now + MIN_REPLY_TIMEOUT,
            },
        };
        self.pending_request = Some(request);
        self.set_state(MasterState::AnswerDataRequest);
        self.protocol_deadline = Some(request.decide_by);
        if self.deliver(frame, Some(id)) {
            Ok(())
        } else {
            self.complete_request(id, ReplyDecision::Postponed, now)
        }
    }

    /// Complete a request-bearing delivery using its generation identifier.
    ///
    /// A [`ReplyDecision::Data`] answer has to arrive before the decision
    /// deadline: T_reply_delay less
    /// [`MstpCoreConfig::reply_transmit_margin`] after the request. A later
    /// one is refused with [`CoreError::RequestExpired`], and the core keeps
    /// the request and sends Reply Postponed when [`Self::on_deadline`] runs.
    /// From 255 ms after the request, the shortest T_reply_timeout, every
    /// decision is refused that way and the request is dropped unanswered
    /// (counted in [`MstpCounters::expired_requests`]).
    pub fn complete_request(
        &mut self,
        id: RequestId,
        decision: ReplyDecision<'_>,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.observe_time(now)?;
        let request = self.pending_request.ok_or(CoreError::NoPendingRequest)?;
        if request.id != id {
            return Err(CoreError::StaleRequest);
        }
        if now >= request.window.give_up {
            self.release_unanswered_request(now);
            return Err(CoreError::RequestExpired);
        }
        let (frame_type, payload) = match decision {
            ReplyDecision::Data(_) if now >= request.decide_by => {
                return Err(CoreError::RequestExpired);
            }
            ReplyDecision::Data(data) if data.len() > MAX_STANDARD_MPDU_DATA => {
                return Err(CoreError::ReplyOversize {
                    length: data.len(),
                    maximum: MAX_STANDARD_MPDU_DATA,
                });
            }
            ReplyDecision::Data(data) => (FrameType::BACnetDataNotExpectingReply, data),
            ReplyDecision::Postponed | ReplyDecision::Cancelled => {
                (FrameType::ReplyPostponed, &[][..])
            }
            ReplyDecision::Abandoned => {
                self.pending_request = None;
                self.set_state(MasterState::Idle);
                self.protocol_deadline = Some(now + self.config.t_no_token);
                return Ok(());
            }
        };
        self.queue_frame_until(
            frame_type,
            request.source,
            payload,
            PostTransmit::Idle,
            now,
            Some(request.window),
        )?;
        self.pending_request = None;
        Ok(())
    }

    /// The decision deadline has passed without an answer: send Reply
    /// Postponed, however late, unless the requester may have stopped waiting.
    pub(super) fn reply_deadline(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(request) = self.pending_request else {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        };
        match self.complete_request(request.id, ReplyDecision::Postponed, now) {
            // Too late to send anything; the request was dropped and counted.
            Err(CoreError::RequestExpired) => Ok(()),
            result => result,
        }
    }

    /// Drop a request unanswered because its requester may have stopped
    /// waiting, and count it.
    pub(super) fn release_unanswered_request(&mut self, now: Instant) {
        self.pending_request = None;
        self.counters.record_expired_request();
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(now + self.config.t_no_token);
    }
}
