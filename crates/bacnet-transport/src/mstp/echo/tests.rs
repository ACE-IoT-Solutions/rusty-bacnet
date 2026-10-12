use super::*;

const FRAME: [u8; 8] = [0x55, 0xFF, 0x00, 0x04, 0x03, 0x00, 0x00, 0x8F];
const GRACE: Duration = Duration::from_millis(40);
const FRAME_ABORT: Duration = Duration::from_millis(100);

fn hunting() -> (EchoFilter, Instant) {
    let now = Instant::now();
    let mut filter = EchoFilter::new(FRAME_ABORT);
    filter.start(&FRAME, now, GRACE);
    (filter, now)
}

/// Push `octets` at `now`; return what reaches the core, and whether the
/// echo was dropped.
fn run(filter: &mut EchoFilter, octets: &[u8], now: Instant) -> (Vec<u8>, bool) {
    let mut fed = Vec::new();
    let mut echo = false;
    for &octet in octets {
        let pushed = filter.push(octet, now);
        fed.extend_from_slice(filter.held(pushed.release));
        if pushed.pass {
            fed.push(octet);
        }
        echo |= pushed.echo;
    }
    (fed, echo)
}

#[test]
fn a_whole_echo_is_dropped_and_what_follows_is_fed() {
    let (mut filter, now) = hunting();
    let mut octets = FRAME.to_vec();
    octets.extend_from_slice(&[0x55, 0xFF, 0x00]);
    assert_eq!(
        run(&mut filter, &octets, now),
        (vec![0x55, 0xFF, 0x00], true)
    );
}

#[test]
fn stray_octets_before_the_echo_pass_on_and_the_echo_is_dropped() {
    for stray in [&[0xFF][..], &[0x55], &[0x55, 0xFF], &[0x55, 0x55, 0xFF]] {
        let (mut filter, now) = hunting();
        let mut octets = stray.to_vec();
        octets.extend_from_slice(&FRAME);
        assert_eq!(
            run(&mut filter, &octets, now),
            (stray.to_vec(), true),
            "stray {stray:02x?}"
        );
    }
}

#[test]
fn a_frame_that_only_starts_like_the_echo_passes_on_whole() {
    let (mut filter, now) = hunting();
    let peer = [0x55, 0xFF, 0x00, 0x03, 0x04, 0x00, 0x00, 0x31];
    assert_eq!(run(&mut filter, &peer, now), (peer.to_vec(), false));
}

#[test]
fn a_short_or_corrupted_echo_passes_on() {
    let (mut filter, now) = hunting();
    assert_eq!(
        run(&mut filter, &FRAME[3..], now),
        (FRAME[3..].to_vec(), false)
    );
    let (mut filter, now) = hunting();
    let mut corrupted = FRAME;
    corrupted[4] ^= 0x40;
    assert_eq!(
        run(&mut filter, &corrupted, now),
        (corrupted.to_vec(), false)
    );
}

#[test]
fn the_first_octet_passed_on_is_a_mismatch_once() {
    let (mut filter, now) = hunting();
    assert!(filter.push(0x01, now).first_mismatch);
    assert!(!filter.push(0x02, now).first_mismatch);
}

#[test]
fn a_grace_with_nothing_held_probes_then_releases() {
    let (mut filter, start) = hunting();
    let end = start + GRACE;
    assert_eq!(filter.deadline(), Some(end));
    assert!(filter.timer(end).missing);
    assert_eq!(filter.deadline(), Some(end + GRACE));
    let pushed = filter.push(0x55, end);
    assert!(!pushed.passes() && !pushed.first_mismatch);
    assert_eq!(
        filter.timer(end + GRACE - Duration::from_millis(1)),
        Ended::default()
    );
    assert_eq!(
        filter.timer(end + GRACE),
        Ended {
            release: 1,
            ..Ended::default()
        }
    );
    assert_eq!(filter.deadline(), None);
    assert!(filter.push(0xFF, end + GRACE).pass);
}

#[test]
fn a_grace_with_part_of_the_echo_held_keeps_hunting() {
    let (mut filter, start) = hunting();
    let end = start + GRACE;
    run(&mut filter, &FRAME[..3], start);
    assert!(!filter.timer(end).missing);
    assert_eq!(filter.deadline(), None);
    assert_eq!(
        run(&mut filter, &FRAME[3..], end + Duration::from_millis(5)),
        (vec![], true)
    );
}

#[test]
fn held_octets_go_stale_after_frame_abort() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..2], start);
    let later = start + FRAME_ABORT + Duration::from_millis(1);
    let pushed = filter.push(0x55, later);
    assert_eq!((pushed.stale, pushed.release, pushed.pass), (2, 0, false));
}

#[test]
fn silence_after_the_grace_ends_the_hunt() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..1], start);
    filter.timer(start + GRACE);
    let later = start + FRAME_ABORT + Duration::from_millis(1);
    let pushed = filter.push(0xFF, later);
    assert_eq!((pushed.stale, pushed.pass), (1, true));
    assert!(filter.push(0x55, later).pass, "the hunt ended");
}

#[test]
fn a_new_start_releases_probe_octets_and_drops_echo_octets() {
    let (mut filter, start) = hunting();
    filter.timer(start + GRACE);
    filter.push(0x55, start + GRACE);
    assert_eq!(filter.before_start(start + GRACE).release, 1);

    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..3], start);
    filter.timer(start + GRACE);
    assert_eq!(filter.before_start(start + GRACE), Ended::default());
}
