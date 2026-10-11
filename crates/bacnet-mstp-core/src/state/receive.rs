//! Receive-frame classification and application delivery.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    /// Abort an expired partial receive and report whether it had progressed
    /// beyond preamble search into a candidate HEADER or DATA frame.
    pub(super) fn abort_partial_if_late(&mut self, now: Instant) -> bool {
        if self.parser_partial
            && self
                .last_octet
                .is_some_and(|last| now - last > self.config.t_frame_abort)
        {
            let invalid_candidate = matches!(
                self.decoder.phase(),
                StreamPhase::Header | StreamPhase::Data
            );
            self.decoder.reset();
            self.parser_partial = false;
            self.counters.record_frame_abort();
            if invalid_candidate {
                self.counters.record_parser_error();
            }
            return invalid_candidate;
        }
        false
    }

    /// Push one octet, then drain the decoder with empty pushes until it needs
    /// more input, as its stream protocol asks, acting on each frame and each
    /// dropped candidate on the way.
    pub(super) fn decode_octet(&mut self, byte: u8, now: Instant) -> Result<(), CoreError> {
        let octet = [byte];
        let mut input: &[u8] = &octet;
        loop {
            let mut frame = None;
            let mut dropped = None;
            match self.decoder.push_slice(input) {
                DecodeEvent::NeedMore => return Ok(()),
                DecodeEvent::Invalid { .. } => {}
                DecodeEvent::Frame { frame: decoded, .. } => {
                    frame = Some(Self::copy_frame(decoded))
                }
                DecodeEvent::Malformed {
                    error, destination, ..
                } => dropped = Some((error, destination)),
                // Only storage too small to hold a header refuses one octet.
                DecodeEvent::Error(error) => dropped = Some((error, None)),
            }
            input = &[];
            if let Some(frame) = frame {
                self.process_owned_frame(frame, now)?;
            }
            if let Some((error, destination)) = dropped {
                if self.dropped_candidate_is_invalid(error, destination) {
                    self.counters.record_parser_error();
                    self.invalid_frame_event(now)?;
                }
            }
        }
    }

    /// Whether a candidate the decoder dropped is an invalid frame for this
    /// station, as the receive state machine of Clause 9.5.4 would see it.
    ///
    /// A header that fails the Clause 9.5.8 checks counts wherever the frame
    /// was going. Trouble found past the header (a length beyond this
    /// station's buffer, an encoding it can't take, a bad data CRC) counts
    /// only on a frame for this station or broadcast (Clause 9.5.4.4); a frame
    /// for another station is skipped without looking at its data.
    ///
    /// The codec refuses a COBS-encoded type before it looks at the source or
    /// the length, so a COBS header with source 255 or a Length outside the
    /// COBS range, which Clause 9.5.8 rejects, looks the same as a good one and
    /// is skipped when it is for another station.
    fn dropped_candidate_is_invalid(&self, error: FrameError, destination: Option<u8>) -> bool {
        match error {
            FrameError::HeaderCrcMismatch
            | FrameError::BroadcastSource
            | FrameError::Oversize { .. } => true,
            // No destination means the header never completed, which the
            // storage `new` accepts rules out; count it to be safe.
            FrameError::FrameTooLong { .. }
            | FrameError::UnsupportedCobsFrame { .. }
            | FrameError::DataCrcMismatch { .. } => destination
                .is_none_or(|address| address == self.config.station || address == BROADCAST_MAC),
            // The stream decoder reports none of these for a dropped
            // candidate; a refused octet (BufferExhausted) is lost input.
            FrameError::TooShort { .. }
            | FrameError::InvalidPreamble { .. }
            | FrameError::UnexpectedData { .. }
            | FrameError::MissingVendorId { .. }
            | FrameError::Truncated { .. }
            | FrameError::BufferExhausted { .. } => true,
        }
    }

    pub(super) fn copy_frame(frame: Frame<'_>) -> OwnedFrame {
        let mut data = [0; MAX_STANDARD_MPDU_DATA];
        data[..frame.data.len()].copy_from_slice(frame.data);
        OwnedFrame {
            header: frame.header,
            length: frame.data.len(),
            data,
        }
    }

    pub(super) fn invalid_frame_event(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.state == MasterState::WaitForReply {
            self.set_state(MasterState::DoneWithToken);
            self.drive_token(now)?;
        } else if self.state == MasterState::PollForMaster {
            self.poll_timeout(now)?;
        }
        Ok(())
    }

    pub(super) fn process_owned_frame(
        &mut self,
        frame: OwnedFrame,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.counters.record_frame_received();
        // While answering a request, before or after the application's
        // decision, this station only sends its reply.
        if self.pending_request.is_some() || self.state == MasterState::AnswerDataRequest {
            return Ok(());
        }
        // A complete valid frame proves that another station began using the
        // token. The octet ingress reaches this transition through
        // Nmin_octets before decoding completes; validated host ingress must
        // apply the same transition explicitly because it arrives as one
        // event rather than as individual octets.
        if self.state == MasterState::PassToken {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
        }
        if self.state == MasterState::WaitForReply {
            return self.process_wait_for_reply(frame, now);
        }
        if self.state == MasterState::PollForMaster {
            if frame.header.frame_type == FrameType::ReplyToPollForMaster
                && frame.header.destination == self.config.station
            {
                self.next_station = frame.header.source;
                self.poll_station = self.config.station;
                self.token_count = 0;
                self.retry_token_count = 0;
                self.sole_master = false;
                return self.queue_token(self.next_station, now);
            }
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        match frame.header.frame_type {
            FrameType::Token if frame.header.destination == self.config.station => {
                self.sole_master = false;
                self.frame_count = 0;
                self.retry_token_count = 0;
                self.set_state(MasterState::UseToken);
                self.drive_token(now)
            }
            FrameType::PollForMaster if frame.header.destination == self.config.station => self
                .queue_frame(
                    FrameType::ReplyToPollForMaster,
                    frame.header.source,
                    &[],
                    PostTransmit::Idle,
                    now,
                ),
            FrameType::BACnetDataNotExpectingReply
                if frame.header.destination == self.config.station
                    || frame.header.destination == BROADCAST_MAC =>
            {
                self.deliver(&frame, None);
                Ok(())
            }
            FrameType::BACnetDataExpectingReply if frame.header.destination == BROADCAST_MAC => {
                self.deliver(&frame, None);
                Ok(())
            }
            FrameType::BACnetDataExpectingReply
                if frame.header.destination == self.config.station =>
            {
                self.begin_request(&frame, now)
            }
            FrameType::TestRequest if frame.header.destination == self.config.station => self
                .queue_frame(
                    FrameType::TestResponse,
                    frame.header.source,
                    &frame.data[..frame.length],
                    PostTransmit::Idle,
                    now,
                ),
            _ => Ok(()),
        }
    }

    /// Clause 9.5.6.4 takes any reply-type frame addressed to this station as
    /// the answer, whichever station sent it; the source is not compared with
    /// the station that was asked.
    fn process_wait_for_reply(&mut self, frame: OwnedFrame, now: Instant) -> Result<(), CoreError> {
        // A Test_Response ends the wait too. It is not delivered yet: the
        // core has no indication for test frames.
        let is_reply = frame.header.destination == self.config.station
            && matches!(
                frame.header.frame_type,
                FrameType::BACnetDataNotExpectingReply
                    | FrameType::TestResponse
                    | FrameType::ReplyPostponed
            );
        if !is_reply {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        if frame.header.frame_type == FrameType::BACnetDataNotExpectingReply {
            self.deliver(&frame, None);
        }
        self.set_state(MasterState::DoneWithToken);
        self.drive_token(now)
    }

    pub(super) fn deliver(&mut self, frame: &OwnedFrame, request_id: Option<RequestId>) -> bool {
        match self.actions.enqueue_delivery(
            frame.header.source,
            frame.header.destination,
            &frame.data[..frame.length],
            request_id,
        ) {
            Ok(()) => {
                self.counters.record_npdu_delivered();
                true
            }
            Err(_) => {
                self.counters.record_action_drop();
                false
            }
        }
    }
}
