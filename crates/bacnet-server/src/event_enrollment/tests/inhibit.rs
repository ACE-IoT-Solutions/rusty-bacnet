//! Event_Algorithm_Inhibit written on an enrollment, with no reference
//! (#1507): the evaluator's gate, pass by pass (Clauses 13.2.2.1 and
//! 13.2.2.1.5).

use super::super::*;
use super::*;
use bacnet_types::constructed::ChangeOfValueCriteria;

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    db.get_mut(&oid)
        .unwrap()
        .write_property(property, None, value, None)
        .unwrap();
}

fn inhibit(db: &mut ObjectDatabase, ee: ObjectIdentifier, on: bool) {
    write(
        db,
        ee,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        PropertyValue::Boolean(on),
    );
}

fn set_value(db: &mut ObjectDatabase, ai: ObjectIdentifier, value: f32) {
    write(
        db,
        ai,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    write(
        db,
        ai,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(value),
    );
}

fn state(db: &ObjectDatabase, ee: ObjectIdentifier) -> EventEnrollmentEvalState {
    db.get(&ee)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap()
}

fn transitions(db: &mut ObjectDatabase) -> Vec<(EventState, EventState)> {
    evaluate_event_enrollments(db, 1)
        .into_iter()
        .map(|t| (t.change.from, t.change.to))
        .collect()
}

/// A written inhibit returns an offnormal enrollment to NORMAL at once,
/// inside pTimeDelayNormal, and the countdown toward NORMAL it was running
/// is gone with it. Inhibited at NORMAL, the high value reports nothing.
#[test]
fn written_inhibit_forces_normal_at_once_and_clears_the_countdown() {
    let (mut db, ee, ai) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    write(
        &mut db,
        ee,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(30),
    );
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
    set_value(&mut db, ai, 50.0);
    assert!(transitions(&mut db).is_empty());
    assert!(state(&db, ee).pending.is_some(), "TO_NORMAL countdown");

    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::HIGH_LIMIT, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).pending, None);

    set_value(&mut db, ai, 85.0);
    for _ in 0..3 {
        assert!(transitions(&mut db).is_empty());
    }
    inhibit(&mut db, ee, false);
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
}

/// A setpoint gap used to end the pass before the algorithm ran; under the
/// inhibit, an offnormal enrollment still returns to NORMAL.
#[test]
fn inhibit_forces_normal_through_a_setpoint_gap() {
    let (mut db, ee, _ai) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    assert_eq!(
        transitions(&mut db),
        [(EventState::NORMAL, EventState::HIGH_LIMIT)]
    );
    let setpoint = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    db.remove(&setpoint).unwrap();
    // Control: without the inhibit the gap changes nothing.
    assert!(transitions(&mut db).is_empty());

    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::HIGH_LIMIT, EventState::NORMAL)]
    );
}

/// The forced NORMAL is a NORMAL indication, so under CHANGE_OF_VALUE it
/// takes the current sample as the baseline (Clause 13.3.3): once the
/// inhibit clears, the unchanged value reports nothing.
#[test]
fn forced_normal_rebaselines_change_of_value() {
    let mut db = ObjectDatabase::new();
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    input.set_present_value(10.0);
    let ai = input.object_identifier();
    db.add(Box::new(input)).unwrap();
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-COV", EventType::CHANGE_OF_VALUE).unwrap();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(1.0),
    });
    enrollment.set_event_enable(EventTransitionBits::all());
    let ee = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    // The first sample is the baseline.
    assert!(transitions(&mut db).is_empty());
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(10.0)));

    // A state the algorithm doesn't name, as a rewrite of Event_Parameters
    // can leave behind.
    db.get_mut(&ee)
        .unwrap()
        .set_event_state_internal(EventState::OFFNORMAL)
        .unwrap();
    set_value(&mut db, ai, 20.0);
    inhibit(&mut db, ee, true);
    assert_eq!(
        transitions(&mut db),
        [(EventState::OFFNORMAL, EventState::NORMAL)]
    );
    assert_eq!(state(&db, ee).cov_baseline, Some(PropertyValue::Real(20.0)));

    inhibit(&mut db, ee, false);
    assert!(
        transitions(&mut db).is_empty(),
        "measured from the baseline before the inhibit"
    );
}
