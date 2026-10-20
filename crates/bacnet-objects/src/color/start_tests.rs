//! The colour objects at the device's start (#1526): In_Progress reads
//! NOT_CONTROLLED until the output is driven; the start drives it to a
//! non-zero default at once, keeps a restored value for a zero default, and
//! leaves an output a write already drove.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetColorCommand;
use bacnet_types::enums::{
    ColorOperation as Op, ColorOperationInProgress as InProgress, PropertyIdentifier as P,
};
use bacnet_types::primitives::ObjectIdentifier;

/// Storage holding one value in memory.
#[derive(Default)]
struct Memory(Mutex<Option<ColorOutputSnapshot>>);

impl Memory {
    fn holding(value: ColorOutputValue) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(ColorOutputSnapshot {
            present_value: value,
        }))))
    }

    fn held(&self) -> Option<ColorOutputValue> {
        self.0.lock().unwrap().map(|saved| saved.present_value)
    }
}

impl ColorOutputPersistence for Memory {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(*self.0.lock().unwrap())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(*snapshot);
        Ok(())
    }
}

fn in_progress(object: &dyn BACnetObject) -> InProgress {
    match object.read_property(P::IN_PROGRESS, None).unwrap() {
        PropertyValue::Enumerated(raw) => InProgress::from_raw(raw),
        other => panic!("In_Progress read as {other:?}"),
    }
}

fn red() -> BACnetXyColor {
    BACnetXyColor::new(0.64, 0.33)
}

fn green() -> BACnetXyColor {
    BACnetXyColor::new(0.3, 0.6)
}

const ZERO: BACnetXyColor = BACnetXyColor::new(0.0, 0.0);

#[test]
fn color_start_drives_a_non_zero_default_at_once() {
    let mut color = ColorObject::new(1, "c").unwrap();
    color.set_default_color(red()).unwrap();
    // A fade Transition doesn't apply: the start isn't a Present_Value write.
    color
        .write_property(P::TRANSITION, None, PropertyValue::Enumerated(1), None)
        .unwrap();
    assert_eq!(in_progress(&color), InProgress::NOT_CONTROLLED);
    color.start_output_internal();
    assert_eq!(in_progress(&color), InProgress::IDLE);
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(red())
    );
    assert_eq!(
        color.read_property(P::TRACKING_VALUE, None).unwrap(),
        xy_value(red())
    );
}

#[test]
fn color_start_with_a_zero_default_and_nothing_restored_stays_undriven() {
    let mut color = ColorObject::new(1, "c").unwrap();
    color.set_default_color(ZERO).unwrap();
    color.start_output();
    assert_eq!(in_progress(&color), InProgress::NOT_CONTROLLED);
    // A STOP with nothing moving drives nothing.
    color
        .set_color_command(BACnetColorCommand::new(Op::STOP))
        .unwrap();
    assert_eq!(in_progress(&color), InProgress::NOT_CONTROLLED);
    color.set_present_value(green()).unwrap();
    assert_eq!(in_progress(&color), InProgress::IDLE);
}

#[test]
fn color_start_with_a_zero_default_keeps_the_restored_colour() {
    let storage = Memory::holding(ColorOutputValue::Xy(green()));
    let mut color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    assert!(color.output_restored());
    color.set_default_color(ZERO).unwrap();
    assert_eq!(in_progress(&color), InProgress::NOT_CONTROLLED);
    color.start_output();
    assert_eq!(in_progress(&color), InProgress::IDLE);
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(green())
    );
    color.wait_for_output_saves();
    assert_eq!(storage.held(), Some(ColorOutputValue::Xy(green())));
}

#[test]
fn color_start_with_a_non_zero_default_replaces_the_restored_colour_and_saves_it() {
    let storage = Memory::holding(ColorOutputValue::Xy(green()));
    let mut color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    color.set_default_color(red()).unwrap();
    color.start_output();
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(red())
    );
    color.wait_for_output_saves();
    assert_eq!(storage.held(), Some(ColorOutputValue::Xy(red())));
}

#[test]
fn color_start_leaves_an_output_a_write_already_drove() {
    let mut color = ColorObject::new(1, "c").unwrap();
    color.set_default_color(red()).unwrap();
    color.set_present_value(green()).unwrap();
    color.start_output();
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(green())
    );
    assert_eq!(in_progress(&color), InProgress::IDLE);
}

#[test]
fn a_cov_snapshot_keeps_not_controlled() {
    let color = ColorObject::new(1, "c").unwrap();
    let snapshot = color.cov_snapshot_internal().unwrap();
    assert_eq!(in_progress(snapshot.as_ref()), InProgress::NOT_CONTROLLED);
}

#[test]
fn temperature_start_drives_a_non_zero_default_clamped_at_once() {
    let mut ct = ColorTemperatureObject::new(1, "ct").unwrap();
    ct.set_default_color_temperature(6500).unwrap();
    ct.set_min_max(2000, 6000).unwrap();
    assert_eq!(ct.default_color_temperature(), 6000);
    // A ramp Transition doesn't apply either.
    ct.write_property(P::TRANSITION, None, PropertyValue::Enumerated(2), None)
        .unwrap();
    assert_eq!(in_progress(&ct), InProgress::NOT_CONTROLLED);
    ct.start_output();
    assert_eq!(in_progress(&ct), InProgress::IDLE);
    assert_eq!(
        ct.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Unsigned(6000)
    );
}

#[test]
fn temperature_start_with_a_zero_default() {
    // Nothing restored: undriven until a command drives it.
    let mut ct = ColorTemperatureObject::new(1, "ct").unwrap();
    ct.set_default_color_temperature(0).unwrap();
    ct.start_output();
    assert_eq!(in_progress(&ct), InProgress::NOT_CONTROLLED);
    let mut step = BACnetColorCommand::new(Op::STEP_UP_CCT);
    step.step_increment = Some(100);
    ct.set_color_command(step).unwrap();
    assert_eq!(in_progress(&ct), InProgress::IDLE);

    // Restored: the start keeps it, clamped to limits set since.
    let storage = Memory::holding(ColorOutputValue::Kelvin(2700));
    let mut ct = ColorTemperatureObject::new(1, "ct")
        .unwrap()
        .with_output_persistence(storage)
        .unwrap();
    ct.set_default_color_temperature(0).unwrap();
    ct.set_min_max(3000, 5000).unwrap();
    ct.start_output();
    assert_eq!(in_progress(&ct), InProgress::IDLE);
    assert_eq!(
        ct.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Unsigned(3000)
    );
}

#[test]
fn temperature_default_setter_refuses_out_of_range() {
    let mut ct = ColorTemperatureObject::new(1, "ct").unwrap();
    assert!(ct.set_default_color_temperature(999).is_err());
    assert!(ct.set_default_color_temperature(30_001).is_err());
    assert_eq!(ct.default_color_temperature(), 4000);
}

/// A fade moves the saved value to its target at once, and a STOP to where
/// the fade stood; a burst of changes lands its newest value.
#[test]
fn saves_follow_present_value() {
    let storage = Arc::new(Memory::default());
    let mut color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    assert!(!color.output_restored());
    let now = Arc::new(Mutex::new(Duration::ZERO));
    let source = Arc::clone(&now);
    color.bind_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    color.set_present_value(ZERO).unwrap();
    let mut fade = BACnetColorCommand::new(Op::FADE_TO_COLOR);
    fade.target_color = Some(BACnetXyColor::new(1.0, 1.0));
    fade.fade_time = Some(1000);
    color.set_color_command(fade).unwrap();
    color.wait_for_output_saves();
    assert_eq!(
        storage.held(),
        Some(ColorOutputValue::Xy(BACnetXyColor::new(1.0, 1.0)))
    );
    *now.lock().unwrap() = Duration::from_millis(500);
    color
        .set_color_command(BACnetColorCommand::new(Op::STOP))
        .unwrap();
    color.wait_for_output_saves();
    assert_eq!(
        storage.held(),
        Some(ColorOutputValue::Xy(BACnetXyColor::new(0.5, 0.5)))
    );
}
