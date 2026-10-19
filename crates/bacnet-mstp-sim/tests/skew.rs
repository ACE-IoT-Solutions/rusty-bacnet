//! Baud and clock error at the edges of the allowed tolerance (#1772).
//!
//! Clause 9.2.3 lets a node's bit rate be one percent off nominal either
//! way, and the simulator holds each node's protocol clock to the same band
//! (see `Skew`). These scenarios put nodes at the edges and check what the
//! master state machine owes its peers on the line's own clock: it answers
//! inside Tusage_delay and Treply_delay, and gives up on a silent successor
//! or replier no sooner than the peer's obligations allow.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant, MstpCoreConfig};
use bacnet_mstp_sim::{
    CollisionClass, Fault, MasterSpec, ReplyPolicy, Sim, SimConfig, SimRng, Skew, WireFrame,
    WireTiming, MAX_SKEW_PPM,
};

use common::*;

/// Clause 9.5.3's limits a node's peers rely on.
const T_USAGE_DELAY: Duration = Duration::from_millis(15);
const T_REPLY_DELAY: Duration = Duration::from_millis(250);

/// `nominal` as a clock `ppm` fast measures it on the line: shorter when
/// fast, longer when slow.
fn on_the_line(nominal: Duration, ppm: i32) -> Duration {
    let scaled = u128::from(nominal.as_nanos()) * 1_000_000 / (1_000_000 + ppm) as u128;
    Duration::from_nanos(u64::try_from(scaled).unwrap())
}

fn close(actual: Duration, expected: Duration, slack: Duration) -> bool {
    actual.max(expected) - actual.min(expected) <= slack
}

/// Clauses 9.5.6.6 (PASS_TOKEN) and 9.2.3: a master whose successor has
/// gone silent retries the Token once its own clock says Tusage_timeout
/// has passed, so a fast clock retries early on the line and a slow one
/// late. Even at the fast edge the wait stays well beyond the Tusage_delay
/// a live successor has to answer in. Its frames run on its own UART's
/// grid.
#[test]
fn token_retries_follow_the_passers_clock_and_still_outwait_tusage_delay() {
    let usage_timeout = MstpCoreConfig::for_baud_rate(BAUD).unwrap().t_usage_timeout;
    for ppm in [-MAX_SKEW_PPM, 0, MAX_SKEW_PPM] {
        let mut sim = Sim::new(SimConfig::new(BAUD));
        sim.add_master(MasterSpec::new(0, 3, BAUD).skew(Skew::common(ppm)));
        sim.add_master(MasterSpec::new(3, 3, BAUD));
        // Station 3 goes silent just after handing station 0 the token.
        run_until_frame(&mut sim, ms(1_000), ms(1_100), |frame| {
            frame.is(FrameType::TOKEN, 3, 0)
        });
        sim.inject(sim.now(), Fault::PowerOff { station: 3 });
        sim.run_until(ms(3_000));

        // The first Token to the silent station that is followed straight
        // away by another: the pass and its retry.
        let to_3 = |frame: &WireFrame| frame.is(FrameType::TOKEN, 0, 3) && frame.is_clean();
        let (first, retry) = sim
            .frames()
            .windows(2)
            .find(|pair| pair[0].start > ms(1_000) && to_3(&pair[0]) && to_3(&pair[1]))
            .map(|pair| (&pair[0], &pair[1]))
            .unwrap_or_else(|| panic!("{ppm} ppm: no retried pass\n{}", log(sim.frames())));
        let waited = retry.start - first.end;
        let expected = on_the_line(usage_timeout, ppm);
        assert!(
            close(waited, expected, Duration::from_nanos(2)),
            "{ppm} ppm: retried after {waited:?}, expected {expected:?}"
        );
        assert!(waited > T_USAGE_DELAY, "{ppm} ppm: {waited:?}");
        // An 8-octet Token on the passer's own bit grid.
        let grid = WireTiming::new(BAUD).with_rate_error(ppm);
        assert_eq!(first.end - first.start, grid.boundary(8), "{ppm} ppm");
    }
}

/// The time from the end of station 0's request to the start of station
/// 3's Reply Postponed, with station 0's and station 3's skews.
fn postponed_after(requester: Skew, responder: Skew) -> Duration {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 3, BAUD).skew(requester));
    sim.add_master(
        MasterSpec::new(3, 3, BAUD)
            .skew(responder)
            .application(ReplyPolicy::Ignore),
    );
    sim.enqueue(ms(1_000), 0, 3, &npdu(true, 7));
    sim.run_until(ms(2_000));
    sim.assert_no_violations();
    let frames = sim.frames();
    let asked = frames
        .iter()
        .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 3))
        .expect("the request went out");
    let answer = &frames[asked + 1];
    assert!(
        answer.is(FrameType::REPLY_POSTPONED, 3, 0),
        "the requester waited for the answer: {}",
        log(frames)
    );
    answer.start - frames[asked].end
}

/// Clauses 9.5.6.9 and 9.5.3: a responder whose application never decides
/// sends Reply Postponed when its own clock reaches the core's decision
/// deadline. With a slow clock and a slow UART that is later on the line,
/// but still inside Treply_delay, and a requester with a fast clock, which
/// gives up soonest, still takes it as its answer.
#[test]
fn reply_postponed_stays_inside_treply_delay_with_a_slow_responder() {
    let nominal = postponed_after(Skew::default(), Skew::default());
    let slow = Skew::common(-MAX_SKEW_PPM);
    let skewed = postponed_after(Skew::common(MAX_SKEW_PPM), slow);
    let expected = on_the_line(nominal, slow.clock_ppm);
    assert!(
        close(skewed, expected, Duration::from_micros(2)),
        "{skewed:?}, expected {expected:?} ({nominal:?} nominal)"
    );
    assert!(skewed > nominal);
    assert!(skewed <= T_REPLY_DELAY, "{skewed:?}");
}

/// A ring of masters at seeded points of the tolerance band, half of them
/// at an edge, carrying requests, answers and broadcasts.
fn skewed_ring(seed: u64) -> (Sim, Vec<u8>, u32) {
    let mut rng = SimRng::new(seed);
    let baud = [9_600, 19_200, 38_400, 76_800, 115_200][rng.below(5) as usize];
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    let mut sim = Sim::new(config);
    let mut stations = vec![0u8, 1, 2, 3, 4, 5, 6, 7];
    while stations.len() > rng.range(3, 6) as usize {
        stations.remove(rng.below(stations.len() as u64) as usize);
    }
    let ppm = |rng: &mut SimRng| -> i32 {
        if rng.chance(50) {
            if rng.chance(50) {
                MAX_SKEW_PPM
            } else {
                -MAX_SKEW_PPM
            }
        } else {
            i32::try_from(rng.range(0, 2 * MAX_SKEW_PPM as u64 + 1)).unwrap() - MAX_SKEW_PPM
        }
    };
    for &station in &stations {
        let skew = Skew {
            baud_ppm: ppm(&mut rng),
            clock_ppm: ppm(&mut rng),
        };
        sim.add_master(MasterSpec::new(station, 7, baud).skew(skew).application(
            ReplyPolicy::Answer {
                delay: Duration::from_millis(rng.range(1, 40)),
                payload: npdu(false, station),
            },
        ));
    }
    for at in (1_000..4_000).step_by(37) {
        let from = stations[rng.below(stations.len() as u64) as usize];
        let to = if rng.chance(20) {
            0xFF
        } else {
            stations[rng.below(stations.len() as u64) as usize]
        };
        if from != to {
            sim.enqueue(ms(at), from, to, &npdu(to != 0xFF, rng.byte()));
        }
    }
    (sim, stations, baud)
}

/// A seeded sweep over the band: every invariant holds (turnaround,
/// Tusage_delay, one token holder, frames that decode and are counted),
/// and once the ring has formed nobody loses or regenerates the token,
/// retries a pass or gives an answer up, and every request is answered in
/// its reply window.
#[test]
fn rings_across_the_tolerance_band_keep_their_timing() {
    for seed in 0..24 {
        let (mut sim, stations, baud) = skewed_ring(seed);
        sim.run_until(ms(900));
        let counters = |sim: &Sim, station| sim.core(station).unwrap().counters();
        let before: Vec<_> = stations.iter().map(|&s| counters(&sim, s)).collect();
        sim.run_until(ms(5_000));
        sim.assert_no_violations();
        assert!(
            sim.report().collisions.iter().all(|c| c.at < ms(900)),
            "seed {seed}: a collision after the ring formed"
        );
        for (&station, before) in stations.iter().zip(&before) {
            let after = counters(&sim, station);
            let context = format!("seed {seed}, {baud} baud, station {station}");
            assert_eq!(
                after.token_regenerations, before.token_regenerations,
                "{context}"
            );
            assert_eq!(after.token_retries, before.token_retries, "{context}");
            assert_eq!(after.expired_requests, 0, "{context}");
            assert_eq!(after.late_replies, 0, "{context}");
        }
        let frames = sim.frames();
        let mut requests = 0;
        for (index, frame) in frames.iter().enumerate() {
            if frame.frame_type() != Some(FrameType::BACNET_DATA_EXPECTING_REPLY)
                || frame.start < Instant::from_millis(900)
                || index + 1 == frames.len()
            {
                continue;
            }
            requests += 1;
            let reply = &frames[index + 1];
            assert!(
                reply.is(
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    frame.destination().unwrap(),
                    frame.source().unwrap()
                ),
                "seed {seed}: {}",
                log(frames)
            );
            assert!(reply.start - frame.end <= T_REPLY_DELAY, "seed {seed}");
        }
        assert!(requests > 20, "seed {seed}: {requests} requests");
    }
}

/// Clause 9.5.6.7 (NO_TOKEN): after a silence each master generates the
/// token in its own Tslot-wide slot past Tno_token, so the lowest address
/// goes first. Two adjacent masters at opposite clock edges see those
/// slots shift by 1% of about half a second each way, which is a whole
/// Tslot between them: station 1's slot opens a tenth of a millisecond
/// before station 0's, so both generate, and their polls collide. The
/// simulator classes it as a generation race, and the ring forms from the
/// next slot on.
#[test]
fn adjacent_masters_at_opposite_clock_edges_race_to_generate_then_form_a_ring() {
    let ring = |slow: i32, fast: i32| {
        let mut sim = Sim::new(SimConfig::new(BAUD));
        sim.add_master(MasterSpec::new(0, 3, BAUD).skew(Skew::common(slow)));
        sim.add_master(MasterSpec::new(1, 3, BAUD).skew(Skew::common(fast)));
        sim.run_until(ms(2_000));
        sim.assert_no_violations();
        sim
    };
    let nominal = ring(0, 0);
    assert!(nominal.report().collisions.is_empty());

    let sim = ring(-MAX_SKEW_PPM, MAX_SKEW_PPM);
    let collisions = &sim.report().collisions;
    assert_eq!(collisions.len(), 1, "{collisions:?}");
    assert_eq!(collisions[0].class, CollisionClass::GenerationRace);
    // Station 0's slot opens at 500 ms on its slow clock: 505.05 ms.
    let station_0_slot = Instant::ZERO + on_the_line(millis(500), -MAX_SKEW_PPM);
    assert!(close(
        collisions[0].at - Instant::ZERO,
        station_0_slot - Instant::ZERO,
        Duration::from_micros(1)
    ));
    let first = &sim.frames()[0];
    assert!(first.collided && first.source() == Some(1), "{first}");
    // The ring forms: both pass the token from then on.
    for (from, to) in [(0, 1), (1, 0)] {
        assert!(sim
            .frames()
            .iter()
            .any(|frame| frame.start > ms(600) && frame.is(FrameType::TOKEN, from, to)));
    }
}
