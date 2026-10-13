//! What the node scenario reports in a good run. `tests/node.rs` compares
//! the native run's summary with it, and the emulation build of the `node`
//! image the emulated CPU's, so the two can't drift apart.

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::node::{
    Admission, AttemptOutcome, Discard, ExchangeCounters, Lease, NodeCounters, RegisterAttempt,
    RegistrationEvent, RegistrationView, SendRefusal,
};
use bacnet_bip_core::{Duration, Instant};

use super::{Summary, BBMD, ORIGIN, PEER, TTL};
use crate::tally::Sent;

/// The hash over every datagram the node sends and where it goes. It was
/// worked out apart from the codec, from the 17 datagrams written out by
/// hand from the Annex J.2 layouts (120 octets): the six NAKs, eight
/// Register-Foreign-Device requests, and three Distribute-Broadcast-To-
/// Network messages.
const SENT_FNV: u32 = 0x8F8B_EFDE;

const fn at(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;
const DBTN_NAK: BvlcResultCode = BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;

/// The summary [`run`](super::run) returns in a good run.
#[must_use]
pub fn expected() -> Summary {
    Summary {
        // A 4-octet header for a unicast; for a broadcast, 6 octets more, the
        // originating address a BBMD adds when it forwards it. The budget of
        // 1507 octets leaves room for 1497 either way.
        max_npdu: [1497, 1497],
        // Each request only a BBMD serves draws its own NAK, sent back to
        // the peer.
        naks: [
            Some((PEER, BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK)),
            Some((PEER, BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK)),
            Some((PEER, BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)),
            Some((PEER, BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK)),
            Some((PEER, BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)),
            Some((PEER, DBTN_NAK)),
        ],
        // Two broadcasts held while the first attempt is out and a third
        // refused; once it succeeded, one sent at once (at 39.99 s).
        admissions: [
            Some(Ok(Admission::Hold)),
            Some(Ok(Admission::Hold)),
            Some(Err(SendRefusal::HoldFull)),
            Some(Ok(Admission::Send)),
        ],
        released: 2,
        // Delivered from where it started, as a broadcast; the stranger's
        // copy is dropped.
        forwarded: Some((ORIGIN, true)),
        untrusted: Some(Discard::ForwardedUntrusted),
        // The DBTN NAK answers none of the node's exchanges, and comes long
        // after the last one ended.
        hint: Some((Some(DBTN_NAK), false)),
        // At 0, and its renewal one 30 s interval into the lease. The DBTN
        // NAK at 40 s brings the next one forward from 60 s to then. Then
        // the BBMD goes quiet: 70 s, and after each 3 s timeout a wait that
        // doubles, 3 s then 6 s, so 76 s and 85 s. The next wait, 12 s, is
        // cut to the renewal after the 70 s attempt, so at 100 s, as the
        // lease lapses, it starts the attempt that is answered. That
        // answer's lease starts at the unanswered 70 s attempt, which it
        // could also be answering, and is due for renewal at once (100.01 s).
        attempts: [
            Some(at(0)),
            Some(at(30_000)),
            Some(at(40_000)),
            Some(at(70_000)),
            Some(at(76_000)),
            Some(at(85_000)),
            Some(at(100_000)),
            Some(at(100_010)),
        ],
        // Each lease runs one time-to-live, 60 s, from the earliest send its
        // success could answer.
        events: [
            Some(RegistrationEvent::Registered {
                since: at(0),
                until: at(60_000),
            }),
            Some(RegistrationEvent::Renewed { until: at(90_000) }),
            Some(RegistrationEvent::Renewed { until: at(100_000) }),
            Some(RegistrationEvent::Unanswered),
            Some(RegistrationEvent::Unanswered),
            Some(RegistrationEvent::Unanswered),
            Some(RegistrationEvent::Lapsed),
            Some(RegistrationEvent::Registered {
                since: at(70_000),
                until: at(130_000),
            }),
            Some(RegistrationEvent::Renewed { until: at(160_000) }),
            None,
        ],
        // At 100.02 s: the last renewal's lease, from the 100 s attempt, and
        // the next renewal one interval into it.
        registration: Some(RegistrationView {
            lease: Some(Lease {
                since: at(100_000),
                until: at(160_000),
            }),
            registered: true,
            lease_remaining: Some(Duration::from_millis(59_980)),
            attempt_in_flight: false,
            next_attempt: Some(at(130_000)),
            last_outcome: Some(AttemptOutcome::Accepted),
            consecutive_failures: 0,
            first_resolved: true,
            held: 0,
        }),
        counters: NodeCounters {
            // Eight attempts sent, five answered with success, three timed
            // out.
            register_foreign_device: ExchangeCounters {
                sent: 8,
                results: 5,
                last_result: Some(SUCCESS),
                timeouts: 3,
                ..ExchangeCounters::default()
            },
            registration_attempts: 8,
            registrations_accepted: 5,
            last_registration_attempt: Some(RegisterAttempt {
                bbmd: BBMD,
                ttl: TTL,
                outcome: AttemptOutcome::Accepted,
                result: Some(SUCCESS),
            }),
            // The DBTN NAK, taken as the hint.
            unmatched_responses: 1,
            last_unmatched_result: Some(DBTN_NAK),
            renewal_hints: 1,
            lapses: 1,
            broadcasts_held: 2,
            broadcasts_released: 2,
            broadcasts_hold_full: 1,
            ..NodeCounters::default()
        },
        // Six NAKs and eight registrations of 6 octets, and three
        // Distribute-Broadcast-To-Network messages of 12.
        sent: Sent {
            count: 17,
            octets: 120,
            fnv: SENT_FNV,
        },
        errors: 0,
    }
}
