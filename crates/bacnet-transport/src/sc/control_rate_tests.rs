use super::*;
use std::time::Duration;

const HUB: ScIngressPath = ScIngressPath::Hub;

fn vmac(id: u16) -> MacAddr {
    let [high, low] = id.to_be_bytes();
    MacAddr::from_slice(&[2, 0, 0, 0, high, low])
}

fn small() -> ControlRate {
    ControlRate::new(ScControlRateLimit {
        burst: 4,
        per_second: 2,
    })
}

#[test]
fn defaults_validate_and_bad_limits_are_refused() {
    assert!(ScControlRateLimit::default().validate().is_ok());
    for (burst, per_second) in [(0, 1), (1, 0), (4097, 1)] {
        assert!(ScControlRateLimit { burst, per_second }.validate().is_err());
    }
    assert!(ScControlRateLimit {
        burst: 4096,
        per_second: 1
    }
    .validate()
    .is_ok());
}

#[test]
fn a_flood_stops_at_the_burst_and_refills_at_the_rate() {
    let mut rate = small();
    let start = Instant::now();
    let a = vmac(1);
    let admitted = (0..10).filter(|_| rate.admit(HUB, &a, start)).count();
    assert_eq!(admitted, 4);
    // Half a second refills one message at 2/s.
    let later = start + Duration::from_millis(500);
    assert!(rate.admit(HUB, &a, later));
    assert!(!rate.admit(HUB, &a, later));
    // A long pause refills only up to the burst.
    let much_later = later + Duration::from_secs(60);
    assert_eq!(
        (0..10).filter(|_| rate.admit(HUB, &a, much_later)).count(),
        4
    );
}

#[test]
fn sources_and_paths_have_their_own_budgets() {
    let mut rate = small();
    let now = Instant::now();
    let (a, b) = (vmac(1), vmac(2));
    while rate.admit(HUB, &a, now) {}
    assert!(rate.admit(HUB, &b, now));
    assert!(rate.admit(ScIngressPath::Direct, &a, now));
}

#[test]
fn the_table_stays_bounded_and_tracked_sources_keep_their_budgets() {
    let mut rate = small();
    let now = Instant::now();
    // Fill the table with sources that have each spent their burst.
    for id in 0..CONTROL_RATE_SOURCES as u16 {
        while rate.admit(HUB, &vmac(id), now) {}
    }
    assert_eq!(rate.tracked(), CONTROL_RATE_SOURCES);
    // New VMACs share one overflow bucket rather than growing the table.
    let newcomers = (1000..1100)
        .filter(|id| rate.admit(HUB, &vmac(*id), now))
        .count();
    assert_eq!(newcomers, 4);
    assert_eq!(rate.tracked(), CONTROL_RATE_SOURCES);
    // A tracked source is still limited by its own bucket, not the overflow.
    assert!(!rate.admit(HUB, &vmac(0), now));
    // Once buckets refill, they are dropped to make room for new sources.
    let later = now + Duration::from_secs(10);
    assert!(rate.admit(HUB, &vmac(2000), later));
    assert_eq!(rate.tracked(), 1);
}

#[test]
fn only_network_messages_are_classified_as_control() {
    assert!(is_network_message(&[0x01, 0x80, 0x00]));
    assert!(is_network_message(&[0x01, 0xa0, 0x00]));
    assert!(!is_network_message(&[0x01, 0x04, 0x00]));
    assert!(!is_network_message(&[0x01, 0x20, 0xff, 0xff, 0x00, 0xff]));
    assert!(!is_network_message(&[0x02, 0x80]));
    assert!(!is_network_message(&[0x01]));
    assert!(!is_network_message(&[]));
}
