//! Counter saturation, which needs the counters set near their limit.
//! Ported from `bacnet-network`'s `router_table_reject_tests.rs` and
//! `router_table_corroboration_tests.rs`.

use super::{ClaimCounters, Reachability, Table};
use crate::{Duration, Instant, Mac, PortId};

type Hosted = Table<256, 1024>;

const BASE: Instant = Instant::from_secs(1_000);
const MAX: u64 = u64::MAX;

fn learned(table: &mut Hosted) {
    table.add_learned(3000, PortId(0), Mac::from_array([1]), BASE);
}

fn claim(table: &mut Hosted, port: u8, now: Instant) -> bool {
    table
        .apply_claim(3000, PortId(port), Mac::from_array([2]), now)
        .applied()
}

#[test]
fn counters_saturate_without_affecting_decisions_and_snapshots_are_owned() {
    let mut table = Hosted::new();
    learned(&mut table);
    table.counters = ClaimCounters {
        reject_applied: MAX - 1,
        reject_dampened: MAX - 1,
        reject_dampened_same_state: MAX - 1,
        reject_dampened_hold_down: MAX - 1,
        learned_ok: MAX - 1,
        learned_cap_ignored: MAX - 1,
        flap_warned: MAX - 1,
        ..Default::default()
    };
    let before = table.claim_counters();
    let now = BASE;
    for _ in 0..2 {
        table.add_learned(3000, PortId(0), Mac::EMPTY, now);
        table.apply_reject(3000, PortId(0), 1, now);
        table.apply_reject(3000, PortId(0), 1, now);
        table.apply_reject(3000, PortId(0), 2, now);
        assert_eq!(
            table.lookup(3000).unwrap().reachability,
            Reachability::Unreachable
        );
        table.record_learned();
        table.record_learning_cap();
        let entry = table.lookup_mut(3000).unwrap();
        entry.flap_count = 2;
        entry.last_port_change = Some(now);
        assert!(table.learn(3000, PortId(1), Mac::EMPTY, now).flapping());
    }
    let counters = table.claim_counters();
    assert_eq!(
        counters,
        ClaimCounters {
            reject_applied: MAX,
            reject_dampened: MAX,
            reject_dampened_same_state: MAX,
            // Each Busy claim lands on a permanently Unreachable route, so it
            // dampens as same-state and never touches the hold-down count.
            reject_dampened_hold_down: MAX - 1,
            learned_ok: MAX,
            learned_cap_ignored: MAX,
            flap_warned: MAX,
            ..Default::default()
        }
    );
    // A clone copies the counts: what happens to the original afterwards
    // doesn't reach it.
    let cloned = table.clone();
    table.record_disconnect_removal_ignored();
    assert_ne!(table.claim_counters(), counters);
    assert_eq!(cloned.claim_counters(), counters);
    assert_eq!(before.reject_applied, MAX - 1);
    assert_eq!(Hosted::default().claim_counters(), ClaimCounters::default());
}

#[test]
fn new_counters_saturate_without_affecting_gate_and_clones_are_independent() {
    let mut table = Hosted::new_hardened();
    learned(&mut table);
    table.counters = ClaimCounters {
        pending_started: MAX - 1,
        corroborated_applied: MAX - 1,
        pending_expired: MAX - 1,
        disconnect_removal_ignored: MAX - 1,
        ..Default::default()
    };
    let now = BASE;
    let before = table.claim_counters();
    for port in 1..=2u8 {
        let start = now + Duration::from_secs(u64::from(port) * 120);
        assert!(!claim(&mut table, port, start));
        let mut cloned = table.clone();
        assert!(claim(&mut cloned, port, start));
        assert!(table.pending_count() > 0);
        assert!(!claim(&mut table, port, start + Duration::from_secs(61)));
        assert!(claim(&mut table, port, start + Duration::from_secs(62)));
        table.record_disconnect_removal_ignored();
    }
    let counters = table.claim_counters();
    assert_eq!(before.pending_started, MAX - 1);
    assert_eq!(
        counters,
        ClaimCounters {
            pending_started: MAX,
            corroborated_applied: MAX,
            pending_expired: MAX,
            disconnect_removal_ignored: MAX,
            ..Default::default()
        }
    );
}

#[test]
fn the_eviction_count_saturates() {
    let mut table: Table<4, 0> = Table::new();
    table.counters.hold_evicted = MAX - 1;
    for network in [10, 20] {
        table.add_learned(network, PortId(0), Mac::EMPTY, BASE);
        table.apply_reject(network, PortId(0), 1, BASE);
    }
    assert_eq!(table.claim_counters().hold_evicted, MAX);
    assert_eq!(table.claim_counters().reject_applied, 2);
}
