//! What the line does: received octets and receive errors from a UART
//! adapter, or octets, frames and invalid frames from a host-side assembler.
//!
//! # Two input paths
//!
//! A UART adapter feeds every octet to [`MasterCore::on_octet`] and every
//! receive error to [`MasterCore::on_receive_error`]; the core's own stream
//! decoder assembles frames and finds invalid ones. A host adapter, whose
//! serial API hands over chunks without reliable per-octet times, assembles
//! frames itself: it reports the raw octets with
//! [`MasterCore::on_host_activity`] as they arrive, then each frame that
//! completes with [`MasterCore::on_validated_frame`] and each that fails
//! with [`MasterCore::on_invalid_frame`]. Both paths drive the same
//! transitions.
//!
//! # EventCount
//!
//! Clause 9.5.4 adds one to EventCount for each octet received and each
//! receive error. `on_octet` and `on_receive_error` count one each, and
//! `on_host_activity` counts the octets it reports. A host frame,
//! `on_validated_frame` or `on_invalid_frame`, counts one more event on top
//! of its octets, so a host adapter must report its octets for the count to
//! match: PASS_TOKEN and NO_TOKEN judge their successor or rival by more than
//! Nmin_octets events (SawTokenUser, SawFrame). A whole valid frame is that
//! evidence on its own, whichever path brings it.
//!
//! # Echo
//!
//! From a confirmed transmit start until it completes, every input is the
//! station's own echo and is ignored, whichever call reports it. Before the
//! start, input is line activity that can push the start back.

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
    /// - **Echo.** Between a confirmed transmit start and its completion the
    ///   frame is the station's own echo and is ignored, as octets are. A
    ///   frame the assembler was part way through when
    ///   [`Self::on_transmit_started`] succeeded can never complete, since
    ///   the receiver is off while the driver is on: drop it there and then,
    ///   as the core drops its own decoder's partial frame. Report
    ///   completion before any frame assembled from what arrived after it.
    /// - **This station's address.** Clause 9 gives a frame whose source is
    ///   this station no rule of its own. Outside a transmission such a frame
    ///   is another station's using the same address, so the core counts it
    ///   in [`MstpCounters::own_address_frames`] as evidence of a duplicate
    ///   and handles it like any other frame. An adapter must therefore never
    ///   feed back its own echo once the transmission has completed.
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
        if self.transmitting() {
            return Ok(());
        }
        self.note_bus_activity(now, 1)?;
        self.process_owned_frame(Self::copy_frame(frame), now)
    }

    /// Report a frame that a host-side assembler found invalid, as the core's
    /// own decoder reports one (Clause 9.5.4): `now` is when the trouble was
    /// found, usually the frame's last octet.
    ///
    /// Report a frame as invalid exactly when Clause 9.5.4 would count it for
    /// this station: a header that fails its checks wherever it was going,
    /// but trouble past the header (a bad data CRC, a length beyond this
    /// station's buffer, an encoding it can't take) only on a frame for this
    /// station or broadcast; skip any other. It counts in
    /// [`MstpCounters::parser_errors`] and as one event of line activity, and
    /// drives the transitions an invalid frame drives: WAIT_FOR_REPLY ends
    /// the token use (InvalidFrame, Clause 9.5.6.4), and POLL_FOR_MASTER
    /// takes the poll as unanswered (SoleMaster, DoneWithPFM, SendNextPFM or
    /// DeclareSoleMaster, Clause 9.5.6.8). In any other state it is line
    /// activity and nothing more: it restarts the line's silence and adds
    /// one to EventCount, so in PASS_TOKEN it can complete the evidence that
    /// the successor took the token (SawTokenUser, Clause 9.5.6.6), and in
    /// NO_TOKEN it ends the wait for a slot, as any activity does. While a
    /// frame of this station's waits to go out, it is only activity for that
    /// frame, which waits for a quiet line again, as for any octet. It is
    /// ignored as echo between a confirmed transmit start and its
    /// completion.
    pub fn on_invalid_frame(&mut self, now: Instant) -> Result<(), CoreError> {
        self.observe_time(now)?;
        if self.transmitting() {
            return Ok(());
        }
        self.note_bus_activity(now, 1)?;
        self.counters.record_parser_error();
        self.invalid_frame_event(now)
    }

    /// Report host-observed bus octets before chunk-tolerant frame assembly.
    ///
    /// Host serial APIs do not preserve reliable per-octet timestamps. This
    /// updates silence and token-use accounting without feeding the exact
    /// inter-octet parser used by [`Self::on_octet`]: each octet is one
    /// event toward EventCount, as an octet fed to `on_octet` is.
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
