//! Token ownership, maintenance polling, and queue scheduling.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    pub(super) fn enter_no_token(&mut self, now: Instant) -> Result<(), CoreError> {
        self.set_state(MasterState::NoToken);
        self.retry_token_count = 0;
        // Clause 9.5.6.2 (LostToken): SawFrame counts events from here.
        self.event_count = 0;
        self.generate_token_if_in_slot(now)
    }

    /// Generate a token in this station's slot of the silence (Clause
    /// 9.5.6.7), or in a later window if the deadline came after the slot
    /// closed: the local recovery policy described on [`MasterCore`], counted
    /// in [`MstpCounters::missed_generation_slots`].
    pub(super) fn generate_token_if_in_slot(&mut self, now: Instant) -> Result<(), CoreError> {
        let cycle = scale(self.config.t_slot, u64::from(self.config.max_master) + 1);
        let first = self.last_activity
            + self.config.t_no_token
            + scale(self.config.t_slot, u64::from(self.config.station));
        let mut start = first;
        if now >= start + self.config.t_slot && cycle != Duration::ZERO {
            let cycles = (now - start).as_micros() / cycle.as_micros();
            start += scale(cycle, cycles);
            if now >= start + self.config.t_slot {
                start += cycle;
            }
        }
        if now < start {
            self.protocol_deadline = Some(start);
            return Ok(());
        }
        if start != first {
            self.counters.record_missed_generation_slot();
        }
        self.counters.record_token_regeneration();
        self.next_station = self.config.station;
        self.token_count = 0;
        self.poll_station = next_addr(self.config.station, self.config.max_master);
        self.queue_poll(Origin::NoToken, now)
    }

    pub(super) fn pass_token_timeout(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.event_count > u32::from(self.config.n_min_octets) {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
        } else if self.retry_token_count < self.config.n_retry_token {
            self.retry_token_count = self.retry_token_count.saturating_add(1);
            self.counters.record_token_retry();
            self.queue_token(self.next_station, Origin::PassToken, now)?;
        } else {
            let failed = self.next_station;
            self.next_station = self.config.station;
            self.poll_station = next_addr(failed, self.config.max_master);
            if self.poll_station == self.config.station {
                self.poll_station = next_addr(self.config.station, self.config.max_master);
            }
            self.retry_token_count = 0;
            self.token_count = 0;
            self.queue_poll(Origin::PassToken, now)?;
        }
        Ok(())
    }

    pub(super) fn poll_timeout(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.sole_master {
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        if self.next_station != self.config.station {
            self.retry_token_count = 0;
            return self.queue_token(self.next_station, Origin::Holder, now);
        }
        let next = next_addr(self.poll_station, self.config.max_master);
        if next == self.config.station {
            self.sole_master = true;
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        self.poll_station = next;
        self.queue_poll(Origin::NextPoll, now)
    }

    pub(super) fn drive_token(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.active_transmit.is_some() || self.pending_send.is_some() {
            return Ok(());
        }
        let frame_limit = u16::from(self.config.max_info_frames)
            .saturating_mul(u16::from(self.config.multiple_frame_ratio))
            .div_ceil(100)
            .clamp(1, u16::from(self.config.max_info_frames)) as u8;
        if self.frame_count < frame_limit && self.dequeue_and_send(now)? {
            return Ok(());
        }
        self.set_state(MasterState::DoneWithToken);
        self.done_with_token(now)
    }

    /// Send the next queued NPDU, if there is one.
    ///
    /// An NPDU that cannot go out in a standard frame is dropped, counted as
    /// oversize, and the next one is tried, so it never holds the token. The
    /// attempts are bounded in case a [`QueueStorage`] fails to drop it.
    fn dequeue_and_send(&mut self, now: Instant) -> Result<bool, CoreError> {
        self.queues.expire(now.as_micros());
        let mut npdu = [0; MAX_STANDARD_MPDU_DATA];
        for _ in 0..=self.queues.total_depth() {
            let Some(kind) = self.next_queue_kind(now) else {
                return Ok(false);
            };
            let item = match self
                .queues
                .dequeue_kind_into(kind, now.as_micros(), &mut npdu)
            {
                Ok(Some(item)) => item,
                Ok(None) => return Ok(false),
                // Too long for the scratch: the queue dropped and counted it.
                Err(_) => continue,
            };
            let Some(data) = npdu.get(..item.len) else {
                // The queue claims more octets than it copied.
                self.record_oversize_npdu(kind);
                continue;
            };
            self.consecutive_broadcast = if kind == QueueKind::Broadcast {
                self.consecutive_broadcast.saturating_add(1)
            } else {
                0
            };
            self.frame_count = self.frame_count.saturating_add(1);
            let expecting = kind == QueueKind::Unicast && data.len() >= 2 && data[1] & 0x04 != 0;
            let (frame_type, post) = if expecting {
                (
                    FrameType::BACnetDataExpectingReply,
                    PostTransmit::WaitForReply,
                )
            } else {
                (
                    FrameType::BACnetDataNotExpectingReply,
                    PostTransmit::ContinueToken,
                )
            };
            self.queue_frame(
                frame_type,
                item.destination,
                data,
                post,
                Origin::Holder,
                now,
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The queue to serve next under the broadcast/unicast ratio.
    fn next_queue_kind(&self, now: Instant) -> Option<QueueKind> {
        let has_u = self
            .queues
            .peek_kind(QueueKind::Unicast, now.as_micros())
            .is_some();
        let has_b = self
            .queues
            .peek_kind(QueueKind::Broadcast, now.as_micros())
            .is_some();
        match (has_u, has_b) {
            (false, false) => None,
            (true, false) => Some(QueueKind::Unicast),
            (false, true) => Some(QueueKind::Broadcast),
            (true, true) if self.consecutive_broadcast < self.config.broadcast_unicast_ratio => {
                Some(QueueKind::Broadcast)
            }
            (true, true) => Some(QueueKind::Unicast),
        }
    }

    fn done_with_token(&mut self, now: Instant) -> Result<(), CoreError> {
        loop {
            let next_ts = next_addr(self.config.station, self.config.max_master);
            let next_ps = next_addr(self.poll_station, self.config.max_master);
            if !self.sole_master && self.next_station == self.config.station {
                self.poll_station = next_ts;
                return self.queue_poll(Origin::Holder, now);
            }
            if self.token_count < self.config.n_poll {
                self.token_count = self.token_count.saturating_add(1);
                if self.sole_master {
                    self.frame_count = 0;
                    self.set_state(MasterState::UseToken);
                    if self.dequeue_and_send(now)? {
                        return Ok(());
                    }
                    continue;
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, Origin::Holder, now);
            }
            if next_ps == self.next_station {
                self.poll_station = self.config.station;
                self.token_count = 1;
                if self.sole_master {
                    self.next_station = self.config.station;
                    self.poll_station = next_ts;
                    return self.queue_poll(Origin::Holder, now);
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, Origin::Holder, now);
            }
            self.poll_station = next_ps;
            return self.queue_poll(Origin::Holder, now);
        }
    }

    /// Pass the token to `destination` and enter PASS_TOKEN; a station that
    /// would pass to itself polls for a successor instead.
    pub(super) fn queue_token(
        &mut self,
        destination: u8,
        origin: Origin,
        now: Instant,
    ) -> Result<(), CoreError> {
        if destination == self.config.station {
            self.poll_station = next_addr(self.config.station, self.config.max_master);
            return self.queue_poll(origin, now);
        }
        self.queue_frame(
            FrameType::Token,
            destination,
            &[],
            PostTransmit::PassToken,
            origin,
            now,
        )?;
        self.set_state(MasterState::PassToken);
        self.event_count = 0;
        Ok(())
    }

    /// Poll `poll_station` for a master and enter POLL_FOR_MASTER.
    fn queue_poll(&mut self, origin: Origin, now: Instant) -> Result<(), CoreError> {
        self.queue_frame(
            FrameType::PollForMaster,
            self.poll_station,
            &[],
            PostTransmit::PollForMaster,
            origin,
            now,
        )?;
        self.set_state(MasterState::PollForMaster);
        Ok(())
    }
}
