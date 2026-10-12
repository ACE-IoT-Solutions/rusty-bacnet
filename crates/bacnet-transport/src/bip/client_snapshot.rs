//! Bounded observations of this transport's outgoing management exchanges.

use std::net::SocketAddrV4;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::node::{AttemptOutcome, ExchangeCounters, NodeCounters, Operation};

/// One management function's locally observed exchanges, cumulative across
/// stop/start on the same transport. Counts saturate at `u64::MAX`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BvlcRequestCounters {
    /// Requests successfully handed to the local UDP socket; not peer receipts.
    pub sent: u64,
    /// Matched, valid Read-BDT-ACK or Read-FDT-ACK messages.
    pub acknowledgements: u64,
    /// Matched BVLC-Result messages carrying success or this function's NAK.
    pub results: u64,
    /// Most recent matched result code, cleared on stop/start.
    pub last_result: Option<BvlcResultCode>,
    /// Sent requests whose response deadline passed without a matched reply.
    pub timeouts: u64,
    /// Calls refused locally because another management exchange owned the slot.
    pub busy: u64,
    /// Admitted requests that failed encoding or local UDP send.
    pub local_errors: u64,
    /// Malformed responses from the expected peer with the expected function.
    /// A malformed Result leaves the request pending; a malformed typed ACK
    /// completes it with its existing payload decode error.
    pub malformed_responses: u64,
}

/// Outcome of the latest admitted Register-Foreign-Device attempt. This
/// describes an exchange, not proof of a currently retained remote FDT entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignRegistrationOutcome {
    /// Admitted locally, without a matched result yet.
    Pending,
    /// The BBMD returned success for the requested TTL (including a zero-TTL
    /// removal sent through the one-shot helper).
    Accepted,
    /// The BBMD returned Register-Foreign-Device-NAK for this attempt. An
    /// earlier accepted entry may still exist.
    Refused,
    /// No matched response arrived by the deadline; remote outcome is unknown.
    TimedOut,
    /// Local encoding/send failed; no remote state is inferred.
    LocalError,
    /// The caller cancelled the exchange; a sent request may still take effect.
    Cancelled,
}

/// Register-Foreign-Device observations for manual and automatic requests.
/// The totals survive stop/start; last-attempt fields and countdown reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForeignRegistrationSnapshot {
    /// Admitted attempts, including local failures; excludes busy refusals.
    pub attempts: u64,
    /// Attempts with a matched success result, not a live lease count.
    pub accepted: u64,
    /// Attempts with a matched registration NAK.
    pub refused: u64,
    /// Peer for the latest admitted attempt, which may be a manual target.
    pub last_bbmd: Option<SocketAddrV4>,
    /// TTL requested by that attempt, unchanged from its wire value.
    pub last_ttl: Option<u16>,
    /// Result for that attempt; absent while pending or after a local failure,
    /// cancellation or timeout. Unknown/unrelated codes are recorded separately.
    pub last_result: Option<BvlcResultCode>,
    /// Outcome of that attempt, cleared when the transport stops or restarts.
    pub last_outcome: Option<ForeignRegistrationOutcome>,
    /// Time until the automatic worker's next scheduled attempt. Busy work or
    /// scheduling delays may postpone it; this is not a remote lease countdown.
    /// Absent outside automatic foreign mode and after stop.
    pub next_attempt_in: Option<Duration>,
}

impl ForeignRegistrationSnapshot {
    pub(super) fn reset_status(&mut self) {
        self.last_bbmd = None;
        self.last_ttl = None;
        self.last_result = None;
        self.last_outcome = None;
        self.next_attempt_in = None;
    }
}

/// Cheap, fixed-size snapshot of this B/IP transport's own BVLC operations.
/// Separate from BBMD-side [`super::ManagementCounters`]. A BVLC exchange has
/// no transaction ID: a late same-peer, same-kind reply can be indistinguishable
/// from a reply to a newer attempt. These are local correlations, not proof of
/// remote execution or a current foreign-device lease.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BvlcClientSnapshot {
    /// Whether the transport has started and has not been stopped/aborted.
    pub running: bool,
    /// Read-Broadcast-Distribution-Table exchanges.
    pub read_bdt: BvlcRequestCounters,
    /// Write-Broadcast-Distribution-Table exchanges.
    pub write_bdt: BvlcRequestCounters,
    /// Read-Foreign-Device-Table exchanges.
    pub read_fdt: BvlcRequestCounters,
    /// Delete-Foreign-Device-Table-Entry exchanges.
    pub delete_fdt_entry: BvlcRequestCounters,
    /// Register-Foreign-Device exchanges, both manual and automatic.
    pub register_foreign_device: BvlcRequestCounters,
    /// Registration-attempt observations and the automatic worker's schedule.
    pub foreign_registration: ForeignRegistrationSnapshot,
    /// Well-formed responses without a matching peer/request, including unknown
    /// Result codes and NAKs for a different operation. They complete no request.
    pub unmatched_responses: u64,
    /// Most recent unclassified BVLC-Result code, including unknown raw codes.
    pub last_unmatched_result: Option<BvlcResultCode>,
    /// Malformed management responses received, including unsolicited ones.
    pub malformed_responses: u64,
}

impl BvlcClientSnapshot {
    pub(super) fn reset_status(&mut self) {
        self.foreign_registration.reset_status();
        self.read_bdt.last_result = None;
        self.write_bdt.last_result = None;
        self.read_fdt.last_result = None;
        self.delete_fdt_entry.last_result = None;
        self.register_foreign_device.last_result = None;
        self.last_unmatched_result = None;
    }

    /// One operation's counters.
    pub(super) fn counters_mut(&mut self, operation: Operation) -> &mut BvlcRequestCounters {
        match operation {
            Operation::ReadBdt => &mut self.read_bdt,
            Operation::WriteBdt => &mut self.write_bdt,
            Operation::ReadFdt => &mut self.read_fdt,
            Operation::DeleteFdtEntry => &mut self.delete_fdt_entry,
            Operation::RegisterForeignDevice => &mut self.register_foreign_device,
        }
    }
}

/// This transport's [`BvlcClientSnapshot`], kept across restarts: the
/// node's counts are added in as it reports them.
#[derive(Debug, Default)]
pub(super) struct ClientRecord {
    snapshot: Mutex<BvlcClientSnapshot>,
}

impl ClientRecord {
    fn lock(&self) -> MutexGuard<'_, BvlcClientSnapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The transport started: clear the last-attempt status.
    pub(super) fn start(&self) {
        let mut snapshot = self.lock();
        snapshot.reset_status();
        snapshot.running = true;
    }

    /// The transport stopped: clear the last-attempt status.
    pub(super) fn stop(&self) {
        let mut snapshot = self.lock();
        snapshot.running = false;
        snapshot.reset_status();
    }

    /// The totals and status so far. The countdown to the next attempt is
    /// the node's to fill in.
    pub(super) fn snapshot(&self) -> BvlcClientSnapshot {
        *self.lock()
    }

    /// Add what the node counted since it last reported.
    pub(super) fn absorb(&self, counters: &NodeCounters) {
        if *counters == NodeCounters::default() {
            return;
        }
        let mut snapshot = self.lock();
        for operation in Operation::ALL {
            add_exchange(
                snapshot.counters_mut(operation),
                counters.exchange(operation),
            );
        }
        let foreign = &mut snapshot.foreign_registration;
        add(&mut foreign.attempts, counters.registration_attempts);
        add(&mut foreign.accepted, counters.registrations_accepted);
        add(&mut foreign.refused, counters.registrations_refused);
        if let Some(attempt) = counters.last_registration_attempt {
            foreign.last_bbmd = Some(attempt.bbmd);
            foreign.last_ttl = Some(attempt.ttl);
            foreign.last_result = attempt.result;
            foreign.last_outcome = Some(outcome(attempt.outcome));
        }
        add(
            &mut snapshot.unmatched_responses,
            counters.unmatched_responses,
        );
        if let Some(code) = counters.last_unmatched_result {
            snapshot.last_unmatched_result = Some(code);
        }
        add(
            &mut snapshot.malformed_responses,
            counters.malformed_responses,
        );
    }
}

fn add_exchange(total: &mut BvlcRequestCounters, delta: &ExchangeCounters) {
    add(&mut total.sent, delta.sent);
    add(&mut total.acknowledgements, delta.acknowledgements);
    add(&mut total.results, delta.results);
    if delta.last_result.is_some() {
        total.last_result = delta.last_result;
    }
    add(&mut total.timeouts, delta.timeouts);
    add(&mut total.busy, delta.busy);
    add(&mut total.local_errors, delta.local_errors);
    add(&mut total.malformed_responses, delta.malformed_responses);
}

fn outcome(outcome: AttemptOutcome) -> ForeignRegistrationOutcome {
    match outcome {
        AttemptOutcome::Pending => ForeignRegistrationOutcome::Pending,
        AttemptOutcome::Accepted => ForeignRegistrationOutcome::Accepted,
        AttemptOutcome::Refused => ForeignRegistrationOutcome::Refused,
        AttemptOutcome::TimedOut => ForeignRegistrationOutcome::TimedOut,
        AttemptOutcome::LocalError => ForeignRegistrationOutcome::LocalError,
        AttemptOutcome::Cancelled => ForeignRegistrationOutcome::Cancelled,
    }
}

/// Add a count the node reported, saturating.
fn add(total: &mut u64, delta: u32) {
    *total = total.saturating_add(u64::from(delta));
}
