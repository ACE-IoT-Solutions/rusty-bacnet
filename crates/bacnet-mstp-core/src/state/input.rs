//! What the line does: received octets and receive errors from a UART
//! adapter, or octets and frames from a host-side assembler.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
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
    /// How this host path treats the station's own echo, frames whose source
    /// is this station, and a frame its assembler was part way through when
    /// a transmit start is confirmed, is still open: #1617 settles them.
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
}
