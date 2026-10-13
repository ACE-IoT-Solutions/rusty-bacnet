//! Each step: what it feeds the core, and what the core must answer.

use bacnet_mstp_core::{
    CoreError, Instant, MasterCore, NetworkPriority, QueueRefusal, QueueStorage, ReceiveError,
    ReplyDecision, RequestId, TransmitFailure, TransmitId,
};

use super::Driver;
use crate::checker::Decided;
use crate::input::{fill, priority, Decision, Echo, Path, Step, TransmitEvent, MAX_OCTETS};
use crate::model::{encode, Seen};

/// How often a deadline may be serviced at one instant before it counts as
/// stuck.
const SERVICES: usize = 16;

impl Driver<'_> {
    pub(super) fn step<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        step: &Step,
    ) {
        let octet_time = self.params().octet_time;
        let line_input = matches!(
            step,
            Step::Octets { .. }
                | Step::Frame { .. }
                | Step::Damaged { .. }
                | Step::Cut { .. }
                | Step::ReceiveError(_)
                | Step::InvalidFrame { .. }
                | Step::HostActivity { .. }
                | Step::Wait(_)
                | Step::Enqueue { .. }
                | Step::Regress { .. }
        );
        if self.lazy && !line_input {
            // Poll what line inputs left before any other call.
            self.drain(core);
        }
        match step {
            Step::Octets { octets, gap } => {
                for &octet in octets.iter().take(MAX_OCTETS) {
                    self.now += octet_time + 16 * u64::from(*gap);
                    self.octet(core, octet);
                }
            }
            Step::Frame { frame, path } => {
                let validated = matches!(path, Path::Validated { .. });
                let frame = self.builder().frame(frame, validated, false);
                match *path {
                    Path::Octets => self.wire(core, &encode(&frame), None),
                    Path::Gapped { at, micros } => {
                        self.wire(core, &encode(&frame), Some((usize::from(at), micros)));
                    }
                    Path::Validated { with_octets } => {
                        let octets = 10 + frame.data.len() as u64;
                        if with_octets {
                            self.now += octet_time;
                            self.line_input(
                                core,
                                |core, now| core.on_host_activity(octets as usize, now),
                                |line, now, _| line.host_activity(octets, now),
                            );
                        }
                        self.now += octets * octet_time;
                        self.validated(core, frame);
                    }
                }
            }
            Step::Damaged {
                frame,
                position,
                bit,
            } => {
                let frame = self.builder().frame(frame, false, true);
                let mut wire = encode(&frame);
                if wire.len() > 8 {
                    let at = 8 + usize::from(*position) % (wire.len() - 8);
                    wire[at] ^= 1 << (bit % 8);
                }
                self.wire(core, &wire, None);
            }
            Step::Cut { frame, at } => {
                let wire = encode(&self.builder().frame(frame, false, false));
                let cut = 1 + usize::from(*at) % (wire.len() - 1);
                self.wire(core, &wire[..cut], None);
            }
            Step::ReceiveError(fault) => {
                self.now += octet_time;
                let error = ReceiveError::from(*fault);
                self.line_input(
                    core,
                    |core, now| core.on_receive_error(error, now),
                    |line, now, heard| line.receive_error(now, heard),
                );
            }
            Step::InvalidFrame { after } => {
                self.now += u64::from(*after);
                self.line_input(
                    core,
                    |core, now| core.on_invalid_frame(now),
                    |line, now, heard| line.invalid(now, heard),
                );
            }
            Step::HostActivity { octets, after } => {
                self.now += u64::from(*after);
                let octets = *octets;
                self.line_input(
                    core,
                    |core, now| core.on_host_activity(usize::from(octets), now),
                    |line, now, _| line.host_activity(u64::from(octets), now),
                );
            }
            Step::Silence(micros) => {
                self.now += u64::from(*micros);
                self.checker.tick(self.now);
                self.expect_ok(core.on_silence(self.at()), "on_silence");
                self.stamp = self.now;
                self.drain(core);
            }
            Step::Wait(micros) => self.now += u64::from(*micros),
            Step::Deadline {
                late,
                start_before_poll,
            } => {
                let due = core
                    .next_deadline()
                    .map_or(self.now, |due| due.as_micros().max(self.now));
                self.now = due + late.micros();
                self.deadline(core, *start_before_poll);
            }
            Step::Start { after } => self.start(core, u64::from(*after)),
            Step::Complete { after, echo } => self.complete(core, u64::from(*after), echo),
            Step::Abort { after, reason } => {
                let Some(tx) = &self.tx else {
                    return self.bogus(core, TransmitEvent::Abort, u32::from(*after));
                };
                let (id, started) = (tx.id, tx.started.is_some());
                self.now += u64::from(*after);
                self.checker.tick(self.now);
                let failure = TransmitFailure::from(*reason);
                self.expect_ok(
                    core.on_transmit_aborted(id, failure, self.at()),
                    "on_transmit_aborted",
                );
                self.stamp = self.now;
                self.tx = None;
                if started {
                    self.checker.aborted(self.now);
                } else {
                    self.checker.withdrawn();
                }
                self.drain(core);
            }
            Step::Bogus { event, id } => self.bogus(core, *event, *id),
            Step::Enqueue {
                to,
                length,
                expecting,
                priority: raw,
                pattern,
                copies,
            } => {
                let max_npdu = self.params().max_npdu;
                let to = self.builder().resolve(*to, false, false);
                let length = if length & 0x8000 != 0 {
                    usize::from(length & 0x7FFF) % (max_npdu + 17)
                } else {
                    usize::from(length % 24)
                };
                let mut npdu = fill(length, *pattern);
                if length >= 2 {
                    npdu[0] = 0x01;
                    npdu[1] = if *expecting { 0x04 } else { 0x00 };
                }
                for _ in 0..=copies % 4 {
                    let result = core.enqueue_npdu(to, &npdu, priority(*raw), self.at());
                    let fine = if length > max_npdu {
                        result == Err(QueueRefusal::Oversize)
                    } else {
                        matches!(result, Ok(()) | Err(QueueRefusal::Full))
                    };
                    if !fine {
                        self.fail(&format!(
                            "an NPDU of {length} octets for {to} was queued with {result:?}"
                        ));
                    }
                }
            }
            Step::Decide {
                pick,
                decision,
                after,
            } => self.decide(core, *pick, *decision, u64::from(*after)),
            Step::Drain => self.drain(core),
            Step::Run { rounds } => {
                for _ in 0..=rounds % 32 {
                    match &self.tx {
                        Some(tx) if tx.started.is_none() => self.start(core, 0),
                        Some(_) => self.complete(core, 0, &Echo::Silent),
                        None => {
                            let Some(due) = core.next_deadline() else {
                                return;
                            };
                            self.now = self.now.max(due.as_micros());
                            self.deadline(core, false);
                        }
                    }
                }
            }
            Step::Regress { back, event } => self.regress(core, *back, *event),
        }
    }

    fn expect_ok(&self, result: Result<(), CoreError>, call: &str) {
        if let Err(error) = result {
            self.fail(&format!("{call} refused: {error:?}"));
        }
    }

    /// Octets back to back, with an optional gap before one of them.
    fn wire<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        octets: &[u8],
        gap: Option<(usize, u32)>,
    ) {
        let octet_time = self.params().octet_time;
        for (index, &octet) in octets.iter().enumerate() {
            self.now += octet_time;
            if let Some((at, micros)) = gap {
                if index == at % octets.len() {
                    self.now += u64::from(micros);
                }
            }
            self.octet(core, octet);
        }
    }

    fn deadline<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        start_before_poll: bool,
    ) {
        self.checker.tick(self.now);
        self.expect_ok(core.on_deadline(self.at()), "on_deadline");
        self.stamp = self.now;
        if start_before_poll {
            let held = self.tx.as_ref().map(|tx| tx.id);
            if let Some(id) = core.snapshot().transmit_id.filter(|&id| Some(id) != held) {
                let result = core.on_transmit_started(id, self.at());
                if result != Err(CoreError::TransmitNotPolled) {
                    self.fail(&format!(
                        "a start before the frame was polled got {result:?}"
                    ));
                }
            }
        }
        self.drain(core);
        self.checker.answers_due(self.now, None);
        for _ in 0..SERVICES {
            match core.next_deadline() {
                Some(due) if due.as_micros() <= self.now => {
                    self.checker.tick(self.now);
                    self.expect_ok(core.on_deadline(self.at()), "on_deadline");
                    self.stamp = self.now;
                    self.drain(core);
                    self.checker.answers_due(self.now, None);
                }
                _ => return,
            }
        }
        self.fail("a deadline stays due however often it is serviced");
    }

    fn start<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        after: u64,
    ) {
        let Some(tx) = &self.tx else {
            return self.bogus(core, TransmitEvent::Start, after as u32);
        };
        let id = tx.id;
        if tx.started.is_some() {
            let before = core.snapshot();
            let result = core.on_transmit_started(id, self.at());
            self.stamp = self.now;
            if result != Err(CoreError::TransmitAlreadyStarted) || core.snapshot() != before {
                self.fail(&format!("a second start got {result:?}"));
            }
            return;
        }
        self.now += after;
        self.checker.tick(self.now);
        let result = core.on_transmit_started(id, self.at());
        self.stamp = self.now;
        match result {
            Ok(()) => {
                self.checker.started(self.now);
                if let Some(tx) = &mut self.tx {
                    tx.started = Some(self.now);
                }
            }
            Err(CoreError::TransmitDeferred) => {
                self.checker.deferred(self.now);
                self.tx = None;
            }
            Err(CoreError::RequestExpired) => {
                self.checker.expired(self.now);
                self.tx = None;
            }
            Err(error) => self.fail(&format!("the start of a polled frame got {error:?}")),
        }
        self.drain(core);
    }

    fn complete<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        after: u64,
        echo: &Echo,
    ) {
        let Some(tx) = &self.tx else {
            return self.bogus(core, TransmitEvent::Complete, after as u32);
        };
        let id = tx.id;
        let Some(start) = tx.started else {
            let before = core.snapshot();
            let result = core.on_transmit_complete(id, self.at());
            self.stamp = self.now;
            if result != Err(CoreError::TransmitNotStarted) || core.snapshot() != before {
                self.fail(&format!("completing an unstarted frame got {result:?}"));
            }
            return;
        };
        let octets = tx.octets.clone();
        let octet_time = self.params().octet_time;
        match echo {
            Echo::Silent => {}
            Echo::Octets => {
                for (index, &octet) in octets.iter().enumerate() {
                    self.now = self.now.max(start + (index as u64 + 1) * octet_time);
                    self.octet(core, octet);
                }
            }
            Echo::Frame => {
                self.now = self.now.max(start + octets.len() as u64 * octet_time);
                let frame = Seen::decode(&octets).unwrap_or_else(|error| self.fail(&error));
                self.validated(core, frame);
            }
            Echo::Noise(noise) => {
                for &octet in noise.iter().take(MAX_OCTETS) {
                    self.now += octet_time;
                    self.octet(core, octet);
                }
            }
        }
        self.now = self.now.max(start + octets.len() as u64 * octet_time) + after;
        self.checker.tick(self.now);
        self.expect_ok(
            core.on_transmit_complete(id, self.at()),
            "on_transmit_complete",
        );
        self.stamp = self.now;
        self.tx = None;
        self.checker.completed(self.now);
        self.drain(core);
    }

    /// A transmit event for an id the adapter doesn't hold, which the core
    /// must refuse as stale without changing anything.
    fn bogus<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        event: TransmitEvent,
        id: u32,
    ) {
        let taken = [
            self.tx.as_ref().map(|tx| tx.id),
            core.snapshot().transmit_id,
        ];
        let mut id = TransmitId::new(id);
        while taken.contains(&Some(id)) {
            id = TransmitId::new(id.generation().wrapping_add(1));
        }
        let before = core.snapshot();
        let at = self.at();
        let result = match event {
            TransmitEvent::Start => core.on_transmit_started(id, at),
            TransmitEvent::Complete => core.on_transmit_complete(id, at),
            TransmitEvent::Abort => core.on_transmit_aborted(id, TransmitFailure::Io, at),
        };
        self.stamp = self.now;
        if result != Err(CoreError::StaleTransmit) || core.snapshot() != before {
            self.fail(&format!("{event:?} for an id nobody holds got {result:?}"));
        }
    }

    fn decide<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        pick: u8,
        decision: Decision,
        after: u64,
    ) {
        let issued = self.requests.len();
        let id = match pick % 4 {
            0 | 1 if issued > 0 => Some(self.requests[issued - 1]),
            2 if issued > 1 => Some(self.requests[issued - 2]),
            _ => None,
        };
        let request = id.unwrap_or(RequestId::new(0xF000_0000 | u32::from(pick)));
        self.now += 8 * after;
        self.checker.tick(self.now);
        let max_npdu = self.params().max_npdu;
        let data = match decision {
            Decision::Data { length, pattern } => fill(
                if length & 0x8000 != 0 {
                    usize::from(length & 0x7FFF) % (max_npdu + 17)
                } else {
                    usize::from(length % 16)
                },
                pattern,
            ),
            _ => Vec::new(),
        };
        let reply = match decision {
            Decision::Data { .. } => ReplyDecision::Data(&data),
            Decision::Postponed | Decision::Cancelled => ReplyDecision::Postponed,
            Decision::Abandoned => ReplyDecision::Abandoned,
        };
        let result = core.complete_request(request, reply, self.at());
        self.stamp = self.now;
        let data_reply = matches!(decision, Decision::Data { .. });
        match result {
            Ok(()) if id.is_none() => self.fail("a decision for a request nobody made was taken"),
            Ok(()) if data_reply && data.len() > max_npdu => {
                self.fail(&format!("a reply of {} octets was taken", data.len()));
            }
            Ok(()) => self.checker.decided(
                request,
                match decision {
                    Decision::Data { .. } => Decided::Data(&data),
                    Decision::Postponed | Decision::Cancelled => Decided::Postponed,
                    Decision::Abandoned => Decided::Abandoned,
                },
            ),
            Err(CoreError::ReplyOversize { .. }) if data.len() <= max_npdu => {
                self.fail(&format!("a reply of {} octets was refused", data.len()));
            }
            Err(CoreError::StaleRequest | CoreError::NoPendingRequest) => {}
            Err(error) if id.is_none() => {
                self.fail(&format!(
                    "a decision for a request nobody made got {error:?}"
                ));
            }
            _ => {}
        }
        self.drain(core);
        if result.is_ok() {
            // A decision the core took makes its answer, and sends it at
            // once if the line is quiet.
            self.checker.answers_due(self.now, Some(request));
        }
    }

    /// A call stamped before the last one: every call refuses it as a clock
    /// regression, and nothing changes.
    fn regress<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        back: u32,
        event: u8,
    ) {
        if self.stamp == 0 {
            return;
        }
        let supplied = Instant::from_micros(self.stamp - 1 - u64::from(back) % self.stamp);
        let before = (core.snapshot(), core.counters());
        let result = match event % 10 {
            0 => core.on_octet(0x55, supplied),
            1 => core.on_receive_error(ReceiveError::Framing, supplied),
            2 => core.on_deadline(supplied),
            3 => core.on_silence(supplied),
            4 => core.on_host_activity(3, supplied),
            5 => core.on_invalid_frame(supplied),
            6 => core.complete_request(RequestId::new(0), ReplyDecision::Postponed, supplied),
            7 => core.on_transmit_started(TransmitId::new(0), supplied),
            8 => core.on_validated_frame(self.token_frame(), supplied),
            _ => match core.enqueue_npdu(1, &[1, 0], NetworkPriority::Normal, supplied) {
                Err(QueueRefusal::Expired) => Err(CoreError::ClockRegression {
                    previous: Instant::from_micros(self.stamp),
                    supplied,
                }),
                other => {
                    self.fail(&format!("an NPDU queued in the past got {other:?}"));
                }
            },
        };
        let expected = Err(CoreError::ClockRegression {
            previous: Instant::from_micros(self.stamp),
            supplied,
        });
        if result != expected || (core.snapshot(), core.counters()) != before {
            self.fail(&format!(
                "a call stamped {} us, before the last at {}, got {result:?}",
                supplied.as_micros(),
                self.stamp
            ));
        }
    }
}
