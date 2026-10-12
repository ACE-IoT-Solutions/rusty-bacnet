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
        // While a frame of this station's waits, its octets were only
        // activity for that frame (see the `pending` module). While one is
        // on the wire, nothing received can be judged: the start dropped any
        // partial frame, so this guard is only a second line.
        if self.pending_send.is_some() || self.active_transmit.is_some() {
            return Ok(());
        }
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
        if self.frame_while_pending(&frame, now)? {
            return Ok(());
        }
        if self.pending_request.is_some() || self.state == MasterState::AnswerDataRequest {
            self.frame_while_busy(&frame, true);
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
                return self.take_successor(frame.header.source, now);
            }
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        match frame.header.frame_type {
            FrameType::Token if frame.header.destination == self.config.station => {
                self.use_received_token(now)
            }
            FrameType::PollForMaster if frame.header.destination == self.config.station => {
                let window = self.poll_reply_window(now);
                self.queue_frame(
                    FrameType::ReplyToPollForMaster,
                    frame.header.source,
                    &[],
                    PostTransmit::Idle,
                    Origin::Answer(window),
                    now,
                )
            }
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
            FrameType::TestRequest if frame.header.destination == self.config.station => {
                let window = self.reply_window(now);
                self.queue_frame(
                    FrameType::TestResponse,
                    frame.header.source,
                    &frame.data[..frame.length],
                    PostTransmit::Idle,
                    Origin::Answer(window),
                    now,
                )
            }
            _ => Ok(()),
        }
    }

    /// A station answered this station's Poll For Master: it becomes the
    /// successor and gets the token (ReceivedReplyToPFM, Clause 9.5.6.8).
    pub(super) fn take_successor(&mut self, source: u8, now: Instant) -> Result<(), CoreError> {
        self.next_station = source;
        self.poll_station = self.config.station;
        self.token_count = 0;
        self.retry_token_count = 0;
        self.sole_master = false;
        self.queue_token(self.next_station, Origin::Holder, now)
    }

    /// Take up a Token addressed to this station (Clause 9.5.6.2).
    pub(super) fn use_received_token(&mut self, now: Instant) -> Result<(), CoreError> {
        self.sole_master = false;
        self.frame_count = 0;
        self.retry_token_count = 0;
        self.set_state(MasterState::UseToken);
        self.drive_token(now)
    }

    /// A valid frame that completes while this station is busy: answering a
    /// request, or waiting to send a frame of its own (Clause 9.5.5.1).
    ///
    /// No transition out of ANSWER_DATA_REQUEST consumes a received frame
    /// (Clause 9.5.6.9), and none can fire while SendFrame waits, so the
    /// standard leaves the frame pending for the state that follows. The core
    /// keeps no second frame buffer for that, and never lets the frame
    /// displace its own:
    ///
    /// - Data that needs no reply from this station is delivered at once.
    /// - A Token for this station is remembered when `keep_token` is set (the
    ///   station answers from IDLE, and goes back there) and taken up on the
    ///   way to IDLE. Otherwise this station already holds the token, and the
    ///   Token is dropped and counted.
    /// - A Poll For Master, data request or Test_Request for this station is
    ///   dropped and counted in [`MstpCounters::discarded_while_busy`]: an
    ///   answer after this station's own frame would come too late, and could
    ///   collide with the station that asked. So is a Reply To Poll For
    ///   Master for it that nothing is waiting for.
    ///
    /// Any frame but delivered data replaces a remembered Token, as a newer
    /// frame replaces a pending one in the receive state machine.
    pub(super) fn frame_while_busy(&mut self, frame: &OwnedFrame, keep_token: bool) {
        let header = frame.header;
        let for_us = header.destination == self.config.station;
        let broadcast = header.destination == BROADCAST_MAC;
        match header.frame_type {
            FrameType::BACnetDataNotExpectingReply if for_us || broadcast => {
                self.deliver(frame, None);
            }
            FrameType::BACnetDataExpectingReply if broadcast => {
                self.deliver(frame, None);
            }
            FrameType::Token if for_us && keep_token => self.pending_token = true,
            FrameType::Token
            | FrameType::PollForMaster
            | FrameType::BACnetDataExpectingReply
            | FrameType::TestRequest
            | FrameType::ReplyToPollForMaster
                if for_us =>
            {
                self.pending_token = false;
                self.counters.record_discarded_while_busy();
            }
            _ => self.pending_token = false,
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
        match self
            .actions
            .enqueue_delivery(frame.header, &frame.data[..frame.length], request_id)
        {
            Ok(()) => {
                self.counters.record_npdu_delivered();
                true
            }
            Err(_) => {
                self.counters.record_delivery_drop();
                false
            }
        }
    }
}
