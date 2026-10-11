//! Core behaviour the simulator shows to be wrong, pinned by tests of the
//! correct behaviour that are ignored until the core is fixed.
//!
//! Gap (#1625): the core's transmit blind spot. From the moment the core hands an
//! adapter a frame (`Action::Transmit`) until the adapter confirms the start
//! (`on_transmit_started`), `on_octet` treats every octet as the station's
//! own echo and drops it. Echo can only begin once the driver is enabled,
//! after the start. An adapter that takes time to start therefore leaves its
//! core deaf: activity in that window does not restart its turnaround, and a
//! frame that arrives in it is lost whole. Clause 9.5.5.1 has the sender wait
//! for Tturnaround of silence immediately before enabling its driver, so the
//! last activity before the start is what counts.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::{AdapterSpec, Fault, MasterSpec, Sim, SimConfig, ViolationKind};

use common::*;

/// A ring of 0 and 3, run until station 3 has just received a token, from
/// when station 3's adapter starts frames `delay` after its core hands them
/// over.
fn slow_station_3(config: SimConfig, delay: Duration) -> Sim {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 3, BAUD))
        .add_master(MasterSpec::new(3, 3, BAUD));
    run_until_frame(&mut sim, ms(1_500), ms(3_000), |frame| {
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
    sim
}

/// Station 3's core hands its next frame over Tturnaround after the token,
/// and its adapter starts it 4 ms later. A noise octet that ends 0.28 ms
/// before that start is activity station 3 must wait Tturnaround after;
/// today it starts on time regardless.
#[test]
#[ignore = "core bug #1625: octets between dispatch and confirmed start are taken for echo, so a slow adapter starts within Tturnaround of activity"]
fn a_slow_adapter_never_starts_within_turnaround_of_activity() {
    let mut sim = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(4));
    let token_end = sim.now();
    sim.inject(
        token_end + Duration::from_micros(4_500),
        Fault::Noise { octets: vec![0x00] },
    );
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
}

/// With a start delay longer than Tusage_timeout, station 0 sends the token
/// a second time before station 3's first frame is out. Station 3's core,
/// waiting for its adapter, drops that Token unheard; it should at least
/// count it as a valid frame received.
#[test]
#[ignore = "core bug #1625: a frame that arrives between dispatch and confirmed start is dropped as echo"]
fn a_frame_that_arrives_before_a_slow_start_is_still_heard() {
    let mut sim = slow_station_3(SimConfig::new(BAUD), Duration::from_millis(25));
    let token_end = sim.now();
    sim.run_until(token_end + millis(100));
    sim.assert_no_violations();
}

/// The same two runs, collecting violations instead of stopping, show the
/// gap today. They pass, and start failing once #1625 is fixed; then delete
/// this test and drop the two ignores above.
#[test]
fn the_blind_spot_today() {
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;

    let mut sim = slow_station_3(config, Duration::from_millis(4));
    let token_end = sim.now();
    sim.inject(
        token_end + Duration::from_micros(4_500),
        Fault::Noise { octets: vec![0x00] },
    );
    sim.run_until(token_end + millis(100));
    let report = sim.report();
    assert_eq!(report.violations[0].kind, ViolationKind::Turnaround);
    assert_eq!(report.blind_spot_starts, 1);

    let mut sim = slow_station_3(config, Duration::from_millis(25));
    let token_end = sim.now();
    sim.run_until(token_end + millis(100));
    let report = sim.report();
    assert_eq!(
        report.violations[0].kind,
        ViolationKind::ListenerMissedFrame
    );
    assert!(report.violations[0].detail.contains("Token 0->3"));
    assert!(report.blind_spot_frames >= 1);
}
