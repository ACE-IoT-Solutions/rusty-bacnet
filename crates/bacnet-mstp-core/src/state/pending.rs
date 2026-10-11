//! Frames this station owes the line but has not started yet.
//!
//! The SendFrame procedure (Clause 9.5.5.1) waits for T_turnaround of quiet
//! line before the driver goes on, and nothing in it abandons the frame. The
//! core keeps such a frame here from the moment a transition produces it
//! until the adapter confirms its start: first while it waits out the
//! turnaround, then, once handed over as [`Action::Transmit`], until
//! [`MasterCore::on_transmit_started`] succeeds. Only then are received
//! octets the station's own echo.
//!
//! Line activity in either phase (an octet, a receive error, host-reported
//! octets or a validated frame) moves the frame's earliest start to
//! T_turnaround after it, and a start reported sooner is refused with
//! [`CoreError::TransmitDeferred`]: the frame waits again and comes back as a
//! new transmit. The state the frame was made for takes none of its own
//! transitions meanwhile, because the transition that made the frame has not
//! finished. Two exceptions end the wait early: activity that is exactly the
//! evidence the sending state watches for, so that state's own transition
//! wins. And every frame has a limit taken from its state; a frame that still
//! cannot start by then is dropped and counted.
//!
//! Every place a frame can wait, with the core state named as Clause 9.5.6
//! names it (events are octets and receive errors, EventCount):
//!
//! | Frame, and the state that sends it | Activity while it waits | Then | Limit, and its counter | Clauses |
//! | --- | --- | --- | --- | --- |
//! | Data reply or Reply Postponed, from ANSWER_DATA_REQUEST | any | waits T_turnaround again; frames that complete meanwhile are handled as while answering a request | the reply window: the asker's shortest T_reply_timeout (255 ms) less one octet and 1 ms; `expired_requests` | 9.5.6.9, 9.5.3 |
//! | Test_Response to a Test_Request, from IDLE | any | as above | as above | 9.5.6.2, 9.5.6.9 |
//! | Reply To Poll For Master, from IDLE | any | as above | the poller's shortest T_usage_timeout (20 ms) less one octet and 1 ms; `expired_requests` | 9.5.6.2, 9.5.6.8, 9.5.3 |
//! | Data frame, from USE_TOKEN (sole master included) | any | waits again; data that needs no answer from this station is delivered, and a Token, Poll For Master, data request or Test_Request for it is dropped (`discarded_while_busy`) | T_no_token after the frame was made; `cancelled_sends`, then IDLE | 9.5.6.3, 9.5.5.1 |
//! | Token, from DONE_WITH_TOKEN or POLL_FOR_MASTER (after a reply, a reply timeout or an invalid reply too) | any | as for a data frame | as for a data frame | 9.5.6.4, 9.5.6.5, 9.5.6.8 |
//! | Poll For Master, from DONE_WITH_TOKEN or POLL_FOR_MASTER | any | as for a data frame | as for a data frame | 9.5.6.5, 9.5.6.8 |
//! | Token retry, or Poll For Master for a new successor, from PASS_TOKEN | up to Nmin_octets events | waits again | T_usage_timeout after the frame was made; `cancelled_sends` | 9.5.6.6 |
//! | the same | more than Nmin_octets events, or a whole valid frame: the successor uses the token (SawTokenUser) | dropped (`cancelled_sends`); IDLE takes up what follows | none | 9.5.6.6 |
//! | Token-generation Poll For Master, from NO_TOKEN | up to Nmin_octets events since NO_TOKEN began | waits again | T_no_token after the frame was made; `cancelled_sends` | 9.5.6.7 |
//! | the same | more than Nmin_octets events, or a whole valid frame: another station is active (SawFrame) | dropped (`cancelled_sends`); IDLE takes up what follows | none | 9.5.6.7 |
//!
//! The last four rows follow a silence of at least T_usage_timeout or
//! T_no_token, so those frames go to the adapter at once; activity can only
//! reach them before an adapter confirms the start. Their event count bounds
//! the wait too: at most Nmin_octets events, each worth up to T_turnaround of
//! delay. That stays far inside T_no_token, so the generating poll's limit is
//! only a backstop; a long T_turnaround can reach PASS_TOKEN's.
//!
//! An answer's limit applies to its start as well: one the adapter starts at
//! or after it is refused with [`CoreError::RequestExpired`], since the asker
//! may be transmitting by then. Other frames' limits bound only waits caused
//! by line activity.

use super::*;

/// What made a frame that waits to go out, which decides what may end the
/// wait early and how long it may last.
#[derive(Debug, Clone, Copy)]
pub(super) enum Origin {
    /// An answer to a frame this station received (a data reply, Reply
    /// Postponed, Test_Response or Reply To Poll For Master), inside the
    /// window its asker waits for it.
    Answer(ReplyWindow),
    /// A frame this station sends while it holds the token.
    Holder,
    /// A token retry, or a poll for a new successor, from PASS_TOKEN.
    PassToken,
    /// The poll that generates a token, from NO_TOKEN.
    NoToken,
}

/// A frame this station has made but not yet started on the wire.
#[derive(Debug, Clone, Copy)]
pub(super) struct PendingSend {
    pub(super) length: usize,
    pub(super) frame: [u8; MAX_STANDARD_FRAME_LENGTH],
    pub(super) post: PostTransmit,
    pub(super) origin: Origin,
    /// T_turnaround after the last line activity.
    pub(super) earliest: Instant,
    /// From here on the frame is dropped rather than sent.
    pub(super) limit: Instant,
}

impl PendingSend {
    /// When the core must look at this frame again: once its turnaround has
    /// passed, or at its limit if that comes first.
    pub(super) fn deadline(&self) -> Instant {
        core::cmp::min(self.earliest, self.limit)
    }
}

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    /// Encode a frame and send it once the line has been quiet for
    /// T_turnaround: at once if it already has, otherwise from the deadline.
    pub(super) fn queue_frame(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        origin: Origin,
        now: Instant,
    ) -> Result<(), CoreError> {
        if self.pending_send.is_some() || self.active_transmit.is_some() {
            return Err(CoreError::ActionCapacity);
        }
        let mut frame = [0; MAX_STANDARD_FRAME_LENGTH];
        let length = Frame::new(frame_type, destination, self.config.station, data)
            .encode_into(&mut frame)?;
        let limit = match origin {
            Origin::Answer(window) => window.give_up,
            Origin::Holder | Origin::NoToken => now + self.config.t_no_token,
            Origin::PassToken => now + self.config.t_usage_timeout,
        };
        self.pending_send = Some(PendingSend {
            length,
            frame,
            post,
            origin,
            earliest: self.last_activity + self.config.t_turnaround,
            limit,
        });
        self.dispatch_pending_send(now)
    }

    /// Hand the waiting frame to the adapter if its turnaround has passed.
    /// It stays here until the adapter confirms the start.
    pub(super) fn dispatch_pending_send(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(());
        };
        // The limit first: under steady line activity the turnaround keeps
        // moving, and the wait must still end.
        if now >= pending.limit {
            return self.drop_pending(now);
        }
        if now < pending.earliest {
            self.protocol_deadline = Some(pending.deadline());
            return Ok(());
        }
        self.dispatch_frame(&pending.frame[..pending.length], pending.post, now)
    }

    /// Line activity while a frame waits, before or after it went to the
    /// adapter: drop it at its limit or on its state's evidence, otherwise
    /// wait T_turnaround from this activity.
    pub(super) fn pending_activity(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(());
        };
        let evidence = matches!(pending.origin, Origin::PassToken | Origin::NoToken)
            && self.event_count > u32::from(self.config.n_min_octets);
        if now >= pending.limit || evidence {
            return self.drop_pending(now);
        }
        let earliest = self.last_activity + self.config.t_turnaround;
        if let Some(pending) = self.pending_send.as_mut() {
            pending.earliest = earliest;
            if self.active_transmit.is_none() {
                self.protocol_deadline = Some(pending.deadline());
            }
        }
        Ok(())
    }

    /// A valid frame completed while a frame of this station's waits. Return
    /// whether the frame was handled here; when not, the core has gone to
    /// IDLE, which takes it up.
    pub(super) fn frame_while_pending(
        &mut self,
        frame: &OwnedFrame,
        now: Instant,
    ) -> Result<bool, CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(false);
        };
        match pending.origin {
            Origin::Answer(_) => self.frame_while_busy(frame, true),
            Origin::Holder => self.frame_while_busy(frame, false),
            // A whole frame is more than Nmin_octets: the successor took the
            // token (SawTokenUser) or another station is active (SawFrame).
            Origin::PassToken | Origin::NoToken => {
                self.drop_pending(now)?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Give up the waiting frame, taking it back from the adapter if it was
    /// handed over, count it, and go to IDLE.
    pub(super) fn drop_pending(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(());
        };
        if let Some(tx) = self.active_transmit.filter(|tx| !tx.started) {
            self.withdraw_transmit(tx.id)?;
        }
        self.pending_send = None;
        match pending.origin {
            Origin::Answer(_) => self.counters.record_expired_request(),
            Origin::Holder | Origin::PassToken | Origin::NoToken => {
                self.counters.record_cancelled_send();
            }
        }
        self.enter_idle(now)
    }
}
