//! Sending a frame the core handed over, and stripping its echo.
//!
//! A frame goes out in four steps, following the core's transmit contract:
//!
//! 1. Octets already waiting in the port are read without blocking and fed
//!    to the core, stamped with the time of the read. They are line
//!    activity, and may make the core take the frame back. The read is
//!    polled once, outside Tokio's cooperative budget; octets that reached
//!    the port after the reactor last looked at it are not seen here, and
//!    are fed after the frame like any later octets.
//! 2. [`MasterCore::on_transmit_started`](bacnet_mstp_core::MasterCore::on_transmit_started); on any refusal the frame is
//!    dropped (a deferred one comes back as a new transmit).
//! 3. [`SerialPort::write`], then [`SerialPort::drain`]. The frame ended
//!    when the drain returned, but never before its time on the wire at the
//!    configured baud rate: a port without a drain is timed that way alone,
//!    and one whose drain returns early (a USB adapter still shifting bits
//!    out) is held to it. A write or drain error aborts the frame.
//! 4. [`MasterCore::on_transmit_complete`](bacnet_mstp_core::MasterCore::on_transmit_complete), stamped with the frame's end.
//!    On a port with [`Echo::Full`] the echo filter (the `echo` module)
//!    first matches what is read against the frame. Completion comes when
//!    the echo has matched, at the first octet that does not match, or when
//!    the grace runs out, whichever is first, and always before the core is
//!    fed anything read after the frame: until completion the core takes
//!    every octet for echo. The filter stays on past a grace that ran out
//!    with part of the echo seen, so its remainder is still dropped; with
//!    none seen, it probes for one more grace (counted in `echo_missing`).
//!
//! Per port kind: `TokioSerialPort` writes to the driver and drains with
//! `tcdrain` on Unix (on Windows it has no drain, so the wire time is used);
//! its echo is whatever the hardware does, `Echo::None` unless configured.
//! Kernel RS-485 is the same port with the kernel switching RTS around each
//! write. `GpioDirectionPort` drains inside its write and holds the driver
//! for its post-transmit delay before switching back, so the frame
//! completes when its write returns; its echo is the inner port's.
//! [`LoopbackSerial`](crate::mstp::LoopbackSerial) delivers at once and is
//! timed by the wire time, echoing only when built with an echo.

use bacnet_mstp_core::TransmitFailure;
use bacnet_types::error::Error;
use tokio::time::Instant;
use tracing::{debug, warn};

use super::{
    increment, poll_once, Core, Echo, Filtered, Mac, Outgoing, ReadFailed, SentKind, SerialPort,
};

/// Reads at most in the non-blocking drain before a start, so a line that
/// never falls quiet cannot hold the task there.
const DRAIN_READS: usize = 8;

impl<S: SerialPort> Mac<S> {
    /// Send the frame the core handed over.
    pub(super) async fn transmit(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        self.drain_input(core).await?;
        let Some(outgoing) = self.outgoing.take() else {
            return Ok(());
        };
        let started = Instant::now();
        if let Err(error) = core.on_transmit_started(outgoing.id, self.clock.at(started)) {
            debug!(?error, "MS/TP transmit start refused");
            return Ok(());
        }
        // Whatever still matched the last frame's echo was that echo.
        self.filter.clear();
        let octets = u32::try_from(self.tx_frame.len()).unwrap_or(u32::MAX);
        let wire_end = started + self.octet_time * octets;
        let sent = match self.serial.write(&self.tx_frame).await {
            Ok(()) => match self.serial.drain().await {
                Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::Unsupported => {
                    Ok(())
                }
                drained => drained,
            },
            Err(error) => {
                // The failed write may have queued part of the frame.
                let _ = self.serial.drain().await;
                Err(error)
            }
        };
        if let Err(error) = sent {
            increment(&self.counts.serial_write_errors);
            warn!("MS/TP write error: {error}");
            let now = self.clock.now();
            if let Err(error) = core.on_transmit_aborted(outgoing.id, TransmitFailure::Io, now) {
                debug!(?error, "MS/TP abort refused");
            }
            return Ok(());
        }
        // The frame ended when the drain returned, or at its wire time if
        // that is later. The timer may wake a little after that; the core is
        // told the end itself.
        let ended = wire_end.max(Instant::now());
        if Instant::now() < ended {
            tokio::time::sleep_until(ended).await;
        }
        match self.echo {
            Echo::None => self.complete(core, &outgoing, ended),
            Echo::Full { grace } => self.strip_echo(core, &outgoing, ended, grace).await?,
        }
        Ok(())
    }

    /// Report the frame complete, stamped with its end.
    fn complete(&mut self, core: &mut Core<'_>, outgoing: &Outgoing, ended: Instant) {
        match core.on_transmit_complete(outgoing.id, self.clock.at(ended)) {
            Ok(()) => self.count_sent(outgoing.kind),
            Err(error) => debug!(?error, "MS/TP completion refused"),
        }
        self.drain_actions(core);
    }

    /// Match the echo of the frame just sent, complete the frame, and feed
    /// the core what follows the echo (see the module docs).
    async fn strip_echo(
        &mut self,
        core: &mut Core<'_>,
        outgoing: &Outgoing,
        ended: Instant,
        grace: std::time::Duration,
    ) -> Result<(), ReadFailed> {
        self.filter.start(&self.tx_frame);
        let grace_end = ended + grace;
        loop {
            let read = tokio::select! {
                biased;
                read = self.serial.read(&mut self.read_buf) => read,
                () = tokio::time::sleep_until(grace_end) => {
                    if !self.filter.seen() {
                        increment(&self.counts.echo_missing);
                        self.filter.probe(grace_end + grace);
                    }
                    self.complete(core, outgoing, ended);
                    return Ok(());
                }
            };
            let count = match read {
                Ok(count) => count,
                Err(error) => {
                    self.read_failed(&error);
                    return Err(ReadFailed);
                }
            };
            let now = self.clock.now();
            for index in 0..count {
                match self.filter.push(self.read_buf[index]) {
                    Filtered::Held => {}
                    Filtered::Echo => {
                        self.complete(core, outgoing, ended);
                        self.receive(core, index + 1..count, now);
                        return Ok(());
                    }
                    Filtered::Release { held, mismatch } => {
                        if mismatch {
                            increment(&self.counts.echo_mismatches);
                        }
                        self.complete(core, outgoing, ended);
                        self.release(core, held, now);
                        self.receive(core, index..count, now);
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Read without waiting and feed the core what is already there, until
    /// nothing is or the core takes the frame back.
    async fn drain_input(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        for _ in 0..DRAIN_READS {
            let Some(read) = poll_once(self.serial.read(&mut self.read_buf)).await else {
                return Ok(());
            };
            match read {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    let now = self.clock.now();
                    self.receive(core, 0..count, now);
                }
                Err(error) => {
                    self.read_failed(&error);
                    return Err(ReadFailed);
                }
            }
            if self.outgoing.is_none() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn count_sent(&self, kind: SentKind) {
        let counter = match kind {
            SentKind::DataExpectingReply => &self.counts.der_tx,
            SentKind::DirectAnswer => &self.counts.dner_tx_direct,
            SentKind::QueuedData => &self.counts.dner_tx_queued,
            SentKind::ReplyPostponed => &self.counts.reply_postponed_tx,
            SentKind::Other => return,
        };
        increment(counter);
    }
}
