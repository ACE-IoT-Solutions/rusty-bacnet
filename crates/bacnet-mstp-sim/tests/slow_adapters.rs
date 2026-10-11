//! Adapters that take a while to start a frame (#1625).
//!
//! From the moment the core hands an adapter a frame (`Action::Transmit`)
//! until the adapter confirms the start (`on_transmit_started`), what the
//! receiver hears is line activity, not echo: echo can only begin once the
//! driver is on. Clause 9.5.5.1 has the sender wait for Tturnaround of
//! silence right before enabling its driver, so the core refuses a start
//! that comes within Tturnaround of that activity, and the frame waits for
//! a quiet line again.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{AdapterSpec, Fault, MasterSpec, Sim, SimConfig};

use common::*;

/// A ring of 0 and 3, run until station 3 has just received a token, from
/// when station 3's adapter starts frames `delay` after its core hands them
/// over. Returns when that token ended.
fn slow_station_3(config: SimConfig, delay: Duration) -> (Sim, Instant) {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 3, BAUD))
        .add_master(MasterSpec::new(3, 3, BAUD));
    let token_end = run_until_frame(&mut sim, ms(1_500), ms(3_000), |frame| {
        frame.is(FrameType::Token, 0, 3)
    });
    let now = sim.now();
    sim.inject(
        now,
        Fault::SetAdapter {
            station: 3,
            adapter: AdapterSpec {
                start_delay: delay,
                ..AdapterSpec::default()
            },
        },
    );
    (sim, token_end)
}

/// Station 3's core hands its next frame over Tturnaround after the token,
/// and its adapter would start it 4 ms later. A noise octet that ends
/// 0.28 ms before that start is activity station 3 must wait Tturnaround
/// after: the core refuses the start, and the frame goes out Tturnaround
/// after the noise, plus the adapter's delay.
#[test]
fn a_slow_adapter_never_starts_within_turnaround_of_activity() {
    let (mut sim, token_end) = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(4));
    let noise_at = token_end + Duration::from_micros(4_500);
    sim.inject(noise_at, Fault::Noise { octets: vec![0x00] });
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
    let noise_end = noise_at + sim.timing().octet();
    let next = first_after(sim.frames(), token_end, |frame| frame.source() == Some(3))
        .expect("station 3's frame");
    assert!(next.is(FrameType::Token, 3, 0), "{next}");
    assert_eq!(
        next.start,
        noise_end + sim.timing().bits(40) + Duration::from_millis(4)
    );
    assert_eq!(sim.report().refused_starts, 1);
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(counters.deferred_starts, 1);
    assert_eq!(counters.cancelled_sends, 0);
}

/// With a start delay longer than Tusage_timeout, station 0 sends the token
/// a second time before station 3's first frame is out. Station 3's core,
/// waiting for its adapter, hears that Token whole and counts it. It already
/// holds the token, so it drops the copy rather than take it twice. The
/// retry ends more than Tturnaround before the adapter's start, so the start
/// goes ahead, and station 0 takes station 3's frame as its successor using
/// the token. Every round of the ring goes the same way.
#[test]
fn a_frame_that_arrives_before_a_slow_start_is_still_heard() {
    let (mut sim, token_end) = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(25));
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
    let frames = sim.frames();
    let retry = first_after(frames, token_end, |_| true).expect("the retry");
    assert!(retry.is(FrameType::Token, 0, 3), "{retry}");
    assert_eq!(retry.start, token_end + millis(20));
    let pass = first_after(frames, retry.end, |_| true).expect("station 3's pass");
    assert!(pass.is(FrameType::Token, 3, 0), "{pass}");
    assert_eq!(
        pass.start,
        token_end + sim.timing().bits(40) + Duration::from_millis(25)
    );
    // One dropped retry for each of station 3's passes.
    let passes = sent_by(frames, 3, FrameType::Token)
        .into_iter()
        .filter(|frame| frame.start > token_end)
        .count();
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(passes, 3);
    assert_eq!(counters.discarded_while_busy, 3);
    assert_eq!(counters.cancelled_sends, 0);
    assert_eq!(sim.report().refused_starts, 0);
}
