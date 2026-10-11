//! Encoded-frame deferral and transmit action dispatch.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize, const FRAME: usize>
    ProfileMasterCore<'rx, Q, NPDU, FRAME>
{
    pub(super) fn queue_frame(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.queue_frame_until(frame_type, destination, data, post, now, None)
    }

    pub(super) fn queue_frame_until(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        now: Instant,
        reply: Option<ReplyWindow>,
    ) -> Result<(), CoreError> {
        let mut encoded = [0; FRAME];
        let length = if data.len() > MAX_STANDARD_MPDU_DATA {
            if NPDU < MAX_EXTENDED_MPDU_DATA {
                return Err(CoreError::ExtendedFrameUnsupported {
                    length: data.len(),
                    maximum: NPDU,
                });
            }
            let extended_type = match frame_type {
                FrameType::BACnetDataExpectingReply => FrameType::ExtendedDataExpectingReply,
                FrameType::BACnetDataNotExpectingReply => FrameType::ExtendedDataNotExpectingReply,
                _ => {
                    return Err(CoreError::NpduOversize {
                        length: data.len(),
                        maximum: MAX_STANDARD_MPDU_DATA,
                    })
                }
            };
            let header = bacnet_mstp_codec::FrameHeader::new(
                extended_type,
                destination,
                self.config.station,
            );
            bacnet_mstp_codec::encode_extended_into(&mut encoded, &header, data)?
        } else {
            let length = Frame::new(frame_type, destination, self.config.station, data)
                .encode_into(&mut encoded)?;
            length
        };
        let earliest = self.last_activity + self.config.t_turnaround;
        if now < earliest {
            let pending = PendingSend {
                length,
                frame: encoded,
                post,
                earliest,
                reply,
            };
            self.protocol_deadline = Some(pending.deadline());
            self.pending_send = Some(pending);
            return Ok(());
        }
        self.dispatch_frame(&encoded[..length], post, reply, now)
    }

    pub(super) fn dispatch_pending_send(&mut self, now: Instant) -> Result<(), CoreError> {
        let pending = self.pending_send.ok_or(CoreError::ActionCapacity)?;
        // The reply window first: under steady line activity the turnaround
        // keeps moving, and the window must still close.
        if pending.reply.is_some_and(|reply| now >= reply.give_up) {
            // The requester may have stopped waiting; sending now could
            // collide with it.
            self.pending_send = None;
            self.counters.record_expired_request();
            return self.enter_idle(now);
        }
        if now < pending.earliest {
            self.protocol_deadline = Some(pending.deadline());
            return Ok(());
        }
        self.dispatch_frame(
            &pending.frame[..pending.length],
            pending.post,
            pending.reply,
            now,
        )?;
        self.pending_send = None;
        Ok(())
    }

    fn dispatch_frame(
        &mut self,
        frame: &[u8],
        post: PostTransmit,
        reply: Option<ReplyWindow>,
        now: Instant,
    ) -> Result<(), CoreError> {
        if self.active_transmit.is_some() {
            return Err(CoreError::ActionCapacity);
        }
        let id = TransmitId::new(self.next_transmit_generation);
        self.actions
            .enqueue_transmission(id, frame)
            .map_err(|error| self.action_error(error))?;
        self.next_transmit_generation = self.next_transmit_generation.wrapping_add(1);
        self.active_transmit = Some(ActiveTransmit {
            id,
            asserted_at: now,
            started: false,
            post,
            reply,
        });
        self.protocol_deadline = None;
        Ok(())
    }

    fn action_error(&mut self, _error: ActionEnqueueError) -> CoreError {
        self.counters.record_action_drop();
        CoreError::ActionCapacity
    }
}
