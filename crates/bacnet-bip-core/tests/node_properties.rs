//! A foreign device against a BBMD made of `bbmd::Fdt`, over a network that
//! loses, delays and duplicates: the lease the node reports never outlasts
//! the BBMD's entry without its grace period, and attempts stay between one
//! response timeout and one renewal interval apart (Annex J.5.2).

mod node_support;

use std::collections::BTreeMap;

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::bbmd::{Fdt, RegistrationLimits, Reservation};
use bacnet_bip_core::node::{Action, Foreign, Node, Permissive, Retry};
use bacnet_bip_core::{Duration, Instant};
use node_support::*;
use proptest::prelude::*;

/// Time-to-live, renewal and response timeout, as the design's traces.
const TTL: u16 = 60;
const TIMEOUT_MS: u64 = 3_000;
const RENEWAL_MS: u64 = 30_000;

/// The longest a copy of a response takes from its request's send: three
/// response timeouts. The engine counts on no more (its answer window).
const MAX_ROUND_TRIP_MS: u64 = 3 * TIMEOUT_MS;

const LIMITS: RegistrationLimits = RegistrationLimits {
    min_ttl: 1,
    max_ttl: u16::MAX,
    max_entries_per_source: 4,
    reserved_capacity: 0,
};

/// What the network and the BBMD do with one attempt.
#[derive(Debug, Clone)]
struct Plan {
    /// The request never arrives.
    request_lost: bool,
    /// How much of the round-trip budget the request takes, per mille.
    request_share: u64,
    /// The BBMD refuses it (table full, say).
    refused: bool,
    /// Each copy of the result: lost, or its share of what is left, per
    /// mille. Two copies are a duplicate.
    copies: Vec<(bool, u64)>,
}

fn plan() -> impl Strategy<Value = Plan> {
    (
        prop::bool::weighted(0.3),
        0u64..=1_000,
        prop::bool::weighted(0.1),
        prop::collection::vec((prop::bool::weighted(0.25), 0u64..=1_000), 1..=2),
    )
        .prop_map(|(request_lost, request_share, refused, copies)| Plan {
            request_lost,
            request_share,
            refused,
            copies,
        })
}

/// Something on its way.
#[derive(Debug, Clone, Copy)]
enum Message {
    /// A Register-Foreign-Device reaches the BBMD; the plan's index.
    Request(usize),
    /// A BVLC-Result reaches the node.
    Result(BvlcResultCode),
}

struct World {
    node: Node,
    fdt: Fdt<2>,
    plans: Vec<Plan>,
    /// Messages by arrival time, then order sent.
    in_flight: BTreeMap<(Instant, u64), Message>,
    sent: u64,
    starts: Vec<Instant>,
    /// Successes counted since the last attempt started.
    accepted_since_start: u32,
    /// Whether the attempt before the newest was accepted, per start.
    accepted_before: Vec<bool>,
}

impl World {
    fn new(retry: Retry, plans: Vec<Plan>) -> Self {
        let foreign = Foreign {
            retry,
            ..Foreign::new(BBMD, TTL)
        };
        assert_eq!(foreign.response_timeout, ms(TIMEOUT_MS));
        assert_eq!(foreign.renewal, ms(RENEWAL_MS));
        let mut world = Self {
            node: Node::new(foreign_link(foreign), Instant::ZERO).unwrap(),
            fdt: Fdt::new(),
            plans,
            in_flight: BTreeMap::new(),
            sent: 0,
            starts: Vec::new(),
            accepted_since_start: 0,
            accepted_before: Vec::new(),
        };
        world.act(Instant::ZERO);
        world
    }

    fn post(&mut self, when: Instant, message: Message) {
        self.sent += 1;
        self.in_flight.insert((when, self.sent), message);
    }

    fn act(&mut self, now: Instant) {
        // A success comes before the renewal it may start at once.
        self.accepted_since_start += self.node.take_counters().registrations_accepted;
        loop {
            match self.node.poll_action() {
                Action::Idle => break,
                Action::Register { exchange, ttl, .. } => {
                    assert_eq!(ttl, TTL);
                    let index = self.starts.len();
                    self.starts.push(now);
                    self.accepted_before.push(self.accepted_since_start > 0);
                    self.accepted_since_start = 0;
                    assert!(!self.node.exchange_sent(exchange, now));
                    let Some(plan) = self.plans.get(index) else {
                        continue;
                    };
                    if !plan.request_lost {
                        let delay = MAX_ROUND_TRIP_MS * plan.request_share / 1_000;
                        self.post(now + ms(delay), Message::Request(index));
                    }
                }
                Action::Registration(_) | Action::ReleaseHeld | Action::DiscardHeld => {}
                Action::ExchangeEnded { .. } => panic!("no manual exchanges here"),
            }
        }
    }

    /// The BBMD takes request `index` at `now` and answers it.
    fn bbmd(&mut self, index: usize, now: Instant) {
        let plan = self.plans[index].clone();
        let outcome = self
            .fdt
            .register(OWN, TTL, now, &LIMITS, Reservation::Ordinary, || {
                !plan.refused
            });
        let code = match outcome.result {
            Ok(_) => BvlcResultCode::SUCCESSFUL_COMPLETION,
            Err(refusal) => refusal.result_code(),
        };
        let used = MAX_ROUND_TRIP_MS * plan.request_share / 1_000;
        for (lost, share) in plan.copies {
            if !lost {
                let delay = (MAX_ROUND_TRIP_MS - used) * share / 1_000;
                self.post(now + ms(delay), Message::Result(code));
            }
        }
    }

    /// The lease the node reports at `now` must end no later than the
    /// BBMD's entry, grace period left out.
    fn check(&self, now: Instant) -> Result<(), TestCaseError> {
        let view = self.node.registration(now).unwrap();
        if !view.registered {
            return Ok(());
        }
        let lease = view.lease.unwrap();
        let entry = self.fdt.get(OWN);
        prop_assert!(
            entry.is_some(),
            "lease {lease:?} at {now:?}, but no FDT entry"
        );
        let entry = entry.unwrap();
        let entry_end = entry.registered_at + Duration::from_secs(u64::from(entry.ttl));
        prop_assert!(
            lease.until <= entry_end,
            "lease {lease:?} at {now:?} outlasts the entry {entry:?}"
        );
        Ok(())
    }

    fn run(&mut self, end: Instant) -> Result<(), TestCaseError> {
        loop {
            let arrival = self.in_flight.keys().next().map(|(when, _)| *when);
            let Some(now) = [arrival, self.node.next_deadline()]
                .into_iter()
                .flatten()
                .min()
                .filter(|now| *now <= end)
            else {
                return Ok(());
            };
            if arrival == Some(now) {
                let (_, message) = self.in_flight.pop_first().unwrap();
                match message {
                    Message::Request(index) => self.bbmd(index, now),
                    Message::Result(code) => {
                        let payload = result(code);
                        self.node
                            .receive(&datagram(&payload, BBMD, UNICAST), now, &mut Permissive);
                    }
                }
            } else {
                self.node.on_deadline(now);
            }
            self.act(now);
            self.check(now)?;
        }
    }
}

proptest! {
    #[test]
    fn lease_never_outlives_the_bbmd_entry_without_grace(
        backoff in any::<bool>(),
        plans in prop::collection::vec(plan(), 48),
    ) {
        let retry = if backoff { Retry::Backoff } else { Retry::AtRenewal };
        let mut world = World::new(retry, plans);
        world.run(at(1_200_000))?;
        let renewal = ms(RENEWAL_MS);
        let timeout = ms(TIMEOUT_MS);
        for (pair, accepted) in world.starts.windows(2).zip(&world.accepted_before[1..]) {
            let gap = pair[1] - pair[0];
            // Never later than one renewal after the attempt before.
            prop_assert!(gap <= renewal, "{pair:?}");
            // Never sooner than one response timeout, but for the one
            // renewal a success that proved an old lease asks for at once.
            prop_assert!(gap >= timeout || *accepted, "{pair:?}");
        }
    }
}
