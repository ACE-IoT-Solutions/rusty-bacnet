//! Logged values named from a Log_DeviceObjectProperty reference, and the
//! fallbacks when it names nothing usable.

use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::{LogColumns, LoggedProperty};
use crate::format::{Datum, Floats, StateTexts, ValueFormat};

fn reference(
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(object_type, instance).unwrap(),
        property.to_raw(),
    )
}

/// References encoded back to back, as a whole read of the property
/// carries them.
fn encoded(references: &[BACnetDeviceObjectPropertyReference]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for reference in references {
        encode_device_object_property_reference(&mut buf, reference);
    }
    buf.to_vec()
}

fn logged(column: Option<LoggedProperty>, value: &PropertyValue) -> String {
    let texts = StateTexts::default();
    ValueFormat::new(ObjectType::TREND_LOG, &texts, Floats::Exact).logged(column, value)
}

fn logged_data(column: Option<LoggedProperty>, value: &PropertyValue) -> Datum {
    let texts = StateTexts::default();
    ValueFormat::new(ObjectType::TREND_LOG, &texts, Floats::Exact).logged_data(column, value)
}

fn status_flags(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits << 4],
    }
}

/// An Event_State log names its enumerations and a Status_Flags log its
/// bits, in text and as data.
#[test]
fn event_state_and_status_flags_logs_are_named() {
    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::EVENT_STATE,
    )]));
    let event_state = columns.column(0);
    assert_eq!(
        event_state,
        Some(LoggedProperty {
            object_type: ObjectType::ANALOG_INPUT,
            property: PropertyIdentifier::EVENT_STATE,
            index: None,
        })
    );
    let offnormal = PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw());
    assert_eq!(logged(event_state, &offnormal), "offnormal");
    assert_eq!(
        logged_data(event_state, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: Some("offnormal".into()),
        }
    );

    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::STATUS_FLAGS,
    )]));
    let flags = columns.column(0);
    // In alarm and out of service: the first and last of the four bits.
    assert_eq!(
        logged(flags, &status_flags(0b1001)),
        "in-alarm | out-of-service"
    );
    assert_eq!(logged(flags, &status_flags(0)), "none");
    assert_eq!(
        logged_data(flags, &status_flags(0b1000)),
        Datum::Bits {
            bits: "1000".into(),
            names: Some(vec!["in-alarm".into()]),
        }
    );
}

/// The monitored object's type names a binary object's state.
#[test]
fn a_binary_present_value_log_names_its_state() {
    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::BINARY_VALUE,
        3,
        PropertyIdentifier::PRESENT_VALUE,
    )]));
    let column = columns.column(0);
    assert_eq!(logged(column, &PropertyValue::Enumerated(1)), "active");
    assert_eq!(logged(column, &PropertyValue::Enumerated(0)), "inactive");
    // A number past the two states names nothing.
    assert_eq!(logged(column, &PropertyValue::Enumerated(7)), "enum 7");
}

/// With no column named, and for a value whose datatype the property
/// doesn't name, the value prints as the log gives it.
#[test]
fn unnamed_and_mismatched_values_print_as_logged() {
    let offnormal = PropertyValue::Enumerated(2);
    assert_eq!(logged(None, &offnormal), "enum 2");
    assert_eq!(logged(None, &status_flags(0b1000)), "bits(1000)");
    assert_eq!(
        logged_data(None, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: None,
        }
    );

    let state = LoggedProperty {
        object_type: ObjectType::ANALOG_INPUT,
        property: PropertyIdentifier::EVENT_STATE,
        index: None,
    };
    let event_state = Some(state);
    assert_eq!(logged(event_state, &PropertyValue::Real(72.5)), "72.5");
    assert_eq!(
        logged(event_state, &status_flags(0b1000)),
        "bits(1000)",
        "Event_State has no named bits"
    );
    let flags = Some(LoggedProperty {
        property: PropertyIdentifier::STATUS_FLAGS,
        ..state
    });
    assert_eq!(logged(flags, &offnormal), "enum 2");
    assert_eq!(
        logged_data(flags, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: None,
        }
    );
}

/// A Trend Log Multiple's array names each member's column; an empty
/// element names nothing, and octets that don't decode name no column.
#[test]
fn columns_follow_the_members_and_fall_back_on_bad_octets() {
    let empty = reference(
        ObjectType::ANALOG_INPUT,
        ObjectIdentifier::WILDCARD_INSTANCE,
        PropertyIdentifier::PRESENT_VALUE,
    );
    let columns = LogColumns::decode(&encoded(&[
        reference(
            ObjectType::ANALOG_INPUT,
            1,
            PropertyIdentifier::PRESENT_VALUE,
        ),
        empty.clone(),
        reference(
            ObjectType::BINARY_INPUT,
            2,
            PropertyIdentifier::PRESENT_VALUE,
        ),
    ]));
    assert!(!columns.is_empty());
    assert_eq!(
        columns.column(0).map(|column| column.object_type),
        Some(ObjectType::ANALOG_INPUT)
    );
    assert_eq!(columns.column(1), None);
    assert_eq!(
        logged(columns.column(2), &PropertyValue::Enumerated(1)),
        "active"
    );
    assert_eq!(columns.column(3), None, "past the last member");

    assert!(LogColumns::decode(&encoded(&[empty])).is_empty());
    assert!(LogColumns::decode(&[]).is_empty());
    let mut cut = encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::EVENT_STATE,
    )]);
    cut.pop();
    assert_eq!(LogColumns::decode(&cut), LogColumns::default());
    // A good reference before the cut one names nothing either: the
    // columns would no longer line up with the members.
    let mut tail = encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::STATUS_FLAGS,
    )]);
    tail.extend_from_slice(&cut);
    assert_eq!(LogColumns::decode(&tail), LogColumns::default());
    // An application-tagged value is no reference.
    assert_eq!(LogColumns::decode(&[0x91, 0x02]), LogColumns::default());
}

/// Which logs have a Log_DeviceObjectProperty to read.
#[test]
fn only_trend_logs_name_their_columns() {
    assert!(LogColumns::applies(ObjectType::TREND_LOG));
    assert!(LogColumns::applies(ObjectType::TREND_LOG_MULTIPLE));
    assert!(!LogColumns::applies(ObjectType::EVENT_LOG));
    assert!(!LogColumns::applies(ObjectType::AUDIT_LOG));
}
