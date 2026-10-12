//! Route discovery's bounds: cooldown, cap, expiry and cancel. The first
//! case is ported from `bacnet-network`'s `router/rb06_convergence_tests.rs`.

use bacnet_router_core::discovery::Discovery;
use bacnet_router_core::{Duration, Instant};

const BASE: Instant = Instant::from_secs(1_000);

const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn expire<const D: usize>(discovery: &mut Discovery<D>, now: Instant) -> Vec<u16> {
    let mut expired = Vec::new();
    let count = discovery.expire(now, |network| expired.push(network));
    assert_eq!(count, expired.len());
    expired
}

#[test]
fn discovery_coalesces_rate_limits_expires_and_caps() {
    let mut disc: Discovery<256> = Discovery::new();
    assert!(disc.should_solicit(3000, BASE));
    assert!(!disc.should_solicit(3000, BASE + secs(1)));
    assert_eq!(disc.len(), 1);
    assert!(disc.should_solicit(3000, BASE + secs(5)));
    assert!(!disc.should_solicit(0, BASE));
    assert!(!disc.should_solicit(0xFFFF, BASE));
    for net in 4000..(4000 + 255) {
        assert!(disc.should_solicit(net, BASE));
    }
    assert_eq!(disc.len(), 256);
    assert!(!disc.should_solicit(9999, BASE));
    assert_eq!(expire(&mut disc, BASE + secs(36)).len(), 256);
    assert_eq!(disc.len(), 0);
    assert!(disc.should_solicit(9999, BASE + secs(37)));
    disc.cancel();
    assert!(disc.is_cancelled());
    assert_eq!(disc.len(), 0);
    assert!(!disc.should_solicit(3000, BASE + secs(100)));
}

#[test]
fn the_cooldown_and_age_are_boundaries() {
    let mut disc: Discovery<4> = Discovery::default();
    assert!(disc.is_empty());
    assert!(disc.should_solicit(9, BASE));
    // Inside the cooldown, nothing moves; at its end, the search restarts.
    assert!(!disc.should_solicit(9, BASE + secs(5) - Duration::from_nanos(1)));
    assert_eq!(disc.last_solicited(9), Some(BASE));
    assert!(disc.should_solicit(9, BASE + secs(5)));
    assert_eq!(disc.last_solicited(9), Some(BASE + secs(5)));
    // The age counts from the latest search, and the age itself still keeps.
    assert!(expire(&mut disc, BASE + secs(35)).is_empty());
    assert_eq!(
        expire(&mut disc, BASE + secs(35) + Duration::from_nanos(1)),
        [9]
    );
    assert_eq!(disc.last_solicited(9), None);
}

#[test]
fn expiry_reports_in_ascending_order_and_keeps_the_rest() {
    let mut disc: Discovery<4> = Discovery::new();
    for (network, at) in [(30, 0), (10, 0), (20, 20), (40, 0)] {
        assert!(disc.should_solicit(network, BASE + secs(at)));
    }
    assert!(!disc.should_solicit(50, BASE));
    assert_eq!(expire(&mut disc, BASE + secs(31)), [10, 30, 40]);
    assert_eq!(disc.len(), 1);
    assert_eq!(disc.last_solicited(20), Some(BASE + secs(20)));
    // Room again for another network.
    assert!(disc.should_solicit(50, BASE + secs(31)));
}

#[test]
fn the_timing_can_be_set() {
    let mut disc: Discovery<4> = Discovery::new();
    disc.set_timing(secs(1), secs(2));
    assert!(disc.should_solicit(9, BASE));
    assert!(disc.should_solicit(9, BASE + secs(1)));
    assert_eq!(
        expire(&mut disc, BASE + secs(3) + Duration::from_nanos(1)),
        [9]
    );
    assert_eq!(Discovery::<4>::COOLDOWN, secs(5));
    assert_eq!(Discovery::<4>::MAX_AGE, secs(30));
}
