//! Duplicate suppression primitive tests.

use bacnet_router_core::LinkAddress;

#[path = "../src/duplicate.rs"]
mod duplicate;

use duplicate::{DuplicateCache, DuplicateCacheStats, DuplicateKey, DuplicateOutcome};

fn address(value: u8) -> LinkAddress {
    LinkAddress::try_from_slice(&[value]).unwrap()
}

fn key(fingerprint: u64) -> DuplicateKey {
    DuplicateKey::new(100, address(1), 200, address(2), fingerprint)
}

#[test]
fn alternate_ingress_has_the_same_canonical_identity() {
    let mut cache = DuplicateCache::<4>::new(100);
    let routed = key(0xfeed);

    assert_eq!(cache.observe(routed, 10), DuplicateOutcome::Inserted);
    // The ingress port and immediate peer are deliberately absent from the
    // key, so a second interface offering the same routed packet is a hit.
    assert_eq!(cache.observe(routed, 11), DuplicateOutcome::Duplicate);
    assert_eq!(
        cache.stats(),
        DuplicateCacheStats {
            occupancy: 1,
            duplicate_hits: 1,
            evictions: 0,
            expirations: 0,
        }
    );
}

#[test]
fn expiration_is_inclusive_and_reclaims_before_matching() {
    let mut cache = DuplicateCache::<2>::new(10);
    let routed = key(1);

    assert_eq!(cache.observe(routed, 100), DuplicateOutcome::Inserted);
    assert_eq!(cache.observe(routed, 109), DuplicateOutcome::Duplicate);
    assert_eq!(cache.observe(routed, 110), DuplicateOutcome::Inserted);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.stats().duplicate_hits, 1);
    assert_eq!(cache.stats().expirations, 1);
}

#[test]
fn full_cache_replaces_in_deterministic_round_robin_order() {
    let mut cache = DuplicateCache::<2>::new(100);
    let first = key(1);
    let second = key(2);
    let third = key(3);
    let fourth = key(4);

    assert_eq!(cache.observe(first, 0), DuplicateOutcome::Inserted);
    assert_eq!(cache.observe(second, 0), DuplicateOutcome::Inserted);
    assert_eq!(cache.observe(third, 1), DuplicateOutcome::Replaced);
    assert_eq!(cache.observe(first, 2), DuplicateOutcome::Replaced);
    assert_eq!(cache.observe(third, 2), DuplicateOutcome::Duplicate);
    assert_eq!(cache.observe(fourth, 3), DuplicateOutcome::Replaced);
    assert_eq!(cache.stats().evictions, 3);
    assert_eq!(cache.len(), 2);
}

#[test]
fn legitimate_repeat_after_expiry_is_new_not_duplicate() {
    let mut cache = DuplicateCache::<1>::new(5);
    let routed = key(0x1234);

    assert_eq!(cache.observe(routed, 50), DuplicateOutcome::Inserted);
    assert_eq!(cache.observe(routed, 54), DuplicateOutcome::Duplicate);
    assert_eq!(cache.observe(routed, 55), DuplicateOutcome::Inserted);
    assert_eq!(cache.observe(routed, 56), DuplicateOutcome::Duplicate);
    assert_eq!(cache.stats().duplicate_hits, 2);
    assert_eq!(cache.stats().expirations, 1);
}

#[test]
fn zero_capacity_never_claims_duplicate_suppression() {
    let mut cache = DuplicateCache::<0>::new(100);
    let routed = key(7);

    assert_eq!(cache.observe(routed, 0), DuplicateOutcome::Untracked);
    assert_eq!(cache.observe(routed, 1), DuplicateOutcome::Untracked);
    assert_eq!(cache.capacity(), 0);
    assert_eq!(cache.len(), 0);
    assert_eq!(cache.stats(), DuplicateCacheStats::default());
}

#[test]
fn zero_retention_is_an_explicit_no_suppression_mode() {
    let mut cache = DuplicateCache::<2>::new(0);
    let routed = key(8);

    assert_eq!(cache.observe(routed, 0), DuplicateOutcome::Untracked);
    assert_eq!(cache.observe(routed, 0), DuplicateOutcome::Untracked);
    assert_eq!(cache.len(), 0);
}
