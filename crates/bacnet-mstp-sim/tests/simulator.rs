//! Checks of the simulator itself: its collision model, trace and failure
//! reporting.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_sim::{
    CollisionClass, Driver, MasterSpec, ScriptSend, SeenFrame, Sim, SimConfig, TraceKind,
    ViolationKind,
};

use common::*;

/// Master 0 alone, with scripted stations 2 and 4 that both answer a poll of
/// station 2, at the same instant.
fn two_answers(config: SimConfig) -> Sim {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 7, BAUD));
    for station in [2, 4] {
        sim.add_scripted(
            station,
            move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
                if frame.destination == 2 && frame.frame_type == FrameType::PollForMaster {
                    sends.push(ScriptSend::frame(
                        millis(1),
                        FrameType::ReplyToPollForMaster,
                        frame.source,
                        station,
                        &[],
                    ));
                }
            },
        );
    }
    sim
}

#[test]
fn overlapping_drivers_collide_and_receivers_hear_garbage() {
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;
    let mut sim = two_answers(config);
    sim.run_until(ms(600));
    let report = sim.report();
    assert_eq!(report.collisions.len(), 1);
    let collision = report.collisions[0];
    assert_eq!(collision.class, CollisionClass::Scripted);
    assert_eq!(
        (collision.driver, collision.other),
        (Driver::Station(4), Driver::Station(2))
    );
    assert_eq!(report.violations[0].kind, ViolationKind::Collision);
    let collided: Vec<Driver> = sim
        .frames()
        .iter()
        .filter(|frame| frame.collided)
        .map(|frame| frame.driver)
        .collect();
    assert_eq!(collided, [Driver::Station(2), Driver::Station(4)]);
    // The master heard eight garbled characters, not a reply, and polled on.
    let after = sim.frames()[4].clone();
    assert!(after.is(FrameType::PollForMaster, 0, 3), "{after}");
    assert!(sim.perturbed());
}

#[test]
fn scripted_collisions_can_be_tolerated() {
    let mut config = SimConfig::new(BAUD);
    config.tolerance.scripted_collisions = true;
    let mut sim = two_answers(config);
    sim.run_until(ms(600));
    assert_eq!(sim.report().collisions_of(CollisionClass::Scripted), 1);
    sim.assert_no_violations();
}

#[test]
#[should_panic(expected = "MS/TP simulator invariant failed")]
fn a_strict_run_stops_at_the_first_violation() {
    let mut sim = two_answers(SimConfig::new(BAUD));
    sim.run_until(ms(600));
}

#[test]
fn the_trace_records_octets_actions_and_state_changes() {
    let mut sim = ring(BAUD, &[0, 3], 3);
    sim.run_until(ms(560));
    let entries: Vec<String> = sim.trace().entries().map(ToString::to_string).collect();
    let has = |text: &str| entries.iter().any(|entry| entry.contains(text));
    assert!(
        has("    500.000ms st000  state Idle -> PollForMaster"),
        "{entries:#?}"
    );
    assert!(has("    500.000ms st000  SetDirection(Transmit)"));
    assert!(has(
        "    500.000ms st000  TX start PollForMaster 0->1 (8 octets)"
    ));
    assert!(has("    500.000ms st000  octet[0] 55"));
    assert!(has("    500.261ms st000  octet[1] FF"));
    assert!(has("st003  state Idle -> NoToken"));
    assert!(sim
        .trace()
        .entries()
        .any(|entry| matches!(entry.kind, TraceKind::FrameEnd(_))));
    let dump = sim.dump(5);
    assert!(dump.starts_with("trace at     560.000ms:\n"), "{dump}");
    assert_eq!(dump.lines().count(), 7);
}
