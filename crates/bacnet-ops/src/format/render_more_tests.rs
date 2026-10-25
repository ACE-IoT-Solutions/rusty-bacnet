//! Indexed reads, array sizes, unmodelled algorithms and the pinned element
//! kinds (the review rounds of #2078 and #2079).

use super::*;

#[test]
fn indexed_reads_are_one_element_and_whole_lists_show_every_element() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::ANALOG_INPUT, &texts, Floats::Exact);
    let stamp = [0x19, 0x07];
    assert_eq!(format.value(P::EVENT_TIME_STAMPS, &stamp), "[seq 7]");
    assert_eq!(
        format.value_at(P::EVENT_TIME_STAMPS, Some(2), &stamp),
        "seq 7"
    );
    assert_eq!(
        format.data_at(P::EVENT_TIME_STAMPS, Some(2), &stamp),
        Datum::Constructed {
            element: "time-stamp".into(),
            text: "seq 7".into()
        }
    );
    // Index 0 is the array's size, an Unsigned.
    assert_eq!(
        format.value_at(P::EVENT_TIME_STAMPS, Some(0), &[0x21, 0x03]),
        "3"
    );
    let many: Vec<u8> = (0..10u8).flat_map(|n| [0x21, n]).collect();
    assert!(format.value(P::DESCRIPTION, &many).ends_with("(10 items)"));
    assert_eq!(
        format.whole_lists().value(P::DESCRIPTION, &many),
        "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]"
    );
}

/// An indexed Weekly_Schedule read names the day it holds, in the text and
/// the typed data (the review of #2078).
#[test]
fn indexed_schedule_days_are_named_for_their_place_in_the_week() {
    use bacnet_encoding::constructed::encode_daily_schedule;
    use bacnet_types::constructed::BACnetTimeValue;
    let day = bytes(|buf| {
        encode_daily_schedule(
            buf,
            &[BACnetTimeValue {
                time: hm(7, 0),
                value: PropertyValue::Real(21.5),
            }],
        )
        .unwrap()
    });
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::SCHEDULE, &texts, Floats::Exact);
    for (index, name) in [(1, "Mon"), (3, "Wed"), (7, "Sun")] {
        let shown = format!("{name} 07:00 = 21.5");
        assert_eq!(
            format.value_at(P::WEEKLY_SCHEDULE, Some(index), &day),
            shown
        );
        assert_eq!(
            format.data_at(P::WEEKLY_SCHEDULE, Some(index), &day),
            Datum::Constructed {
                element: "daily-schedule".into(),
                text: shown
            }
        );
    }
}

/// Index 0 of an array is its size: a count, never named as a state.
#[test]
fn an_array_size_is_a_plain_count() {
    let texts = StateTexts {
        states: vec!["Low".into(), "Medium".into(), "High".into()],
        ..StateTexts::default()
    };
    let format = ValueFormat::new(O::MULTI_STATE_OUTPUT, &texts, Floats::Exact);
    // A size of 2 would read as state 2, Medium, if taken for a state.
    let size = [0x21, 2];
    assert_eq!(format.value_at(P::PRIORITY_ARRAY, Some(0), &size), "2");
    assert_eq!(
        format.data_at(P::PRIORITY_ARRAY, Some(0), &size),
        Datum::Unsigned(2)
    );
    // An element of the array is still a state.
    assert_eq!(
        format.value_at(P::PRIORITY_ARRAY, Some(8), &[0x21, 2]),
        "2 (Medium)"
    );
}

/// An Event_Parameters algorithm with no model shows its fields by context
/// tag instead of hex, and an empty MAC reads as a broadcast.
#[test]
fn unmodelled_algorithms_show_their_fields() {
    // unsigned-range [11]: time-delay [0] 30, low [1] 10, high [2] 300.
    let parameter = BACnetEventParameter::Opaque {
        tag: 11,
        data: vec![0x09, 30, 0x19, 10, 0x2A, 0x01, 0x2C],
    };
    let octets = bytes(|buf| encode_event_parameter(buf, &parameter).unwrap());
    assert_eq!(
        text(O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS, &octets),
        "unsigned-range: [0] [raw 1e], [1] [raw 0a], [2] [raw 01 2c]"
    );
    // The out-of-range forms read their limits with their datatypes: a
    // negative signed limit stays negative.
    for (tag, body, shown) in [
        (
            15,
            vec![0x09, 30, 0x19, 0xFB, 0x29, 40, 0x39, 2],
            "signed-out-of-range: delay 30 s, low -5, high 40, deadband 2",
        ),
        (
            16,
            vec![0x09, 5, 0x19, 10, 0x2A, 0x01, 0x2C, 0x39, 1],
            "unsigned-out-of-range: delay 5 s, low 10, high 300, deadband 1",
        ),
        (
            14,
            [
                vec![0x09, 0, 0x1D, 8],
                (-1.5f64).to_be_bytes().to_vec(),
                vec![0x2D, 8],
                99.25f64.to_be_bytes().to_vec(),
                vec![0x3D, 8],
                0.5f64.to_be_bytes().to_vec(),
            ]
            .concat(),
            "double-out-of-range: delay 0 s, low -1.5, high 99.25, deadband 0.5",
        ),
    ] {
        let parameter = BACnetEventParameter::Opaque { tag, data: body };
        let octets = bytes(|buf| encode_event_parameter(buf, &parameter).unwrap());
        assert_eq!(
            text(O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS, &octets),
            shown
        );
    }
    let broadcast = bytes(|buf| {
        encode_recipient(
            buf,
            &BACnetRecipient::Address(BACnetAddress {
                network_number: 5,
                mac_address: MacAddr::new(),
            }),
        )
        .unwrap()
    });
    assert_eq!(
        text(O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT, &broadcast),
        "network 5 broadcast"
    );
}

/// The JSON names of the element kinds are pinned: renaming an encoding
/// variant must not change them.
#[test]
fn element_kinds_have_fixed_names() {
    let names: Vec<&str> = ConstructedElement::ALL
        .iter()
        .map(|element| constructed::element_name(*element))
        .collect();
    assert_eq!(
        names,
        [
            "name-value",
            "destination",
            "port-permission",
            "read-access-specification",
            "read-access-result",
            "action-list",
            "device-object-reference",
            "authentication-factor-format",
            "authentication-policy",
            "stage-limit-value",
            "access-rule",
            "device-object-property-reference",
            "object-property-reference",
            "property-access-result",
            "recipient",
            "daily-schedule",
            "special-event",
            "calendar-entry",
            "date-range",
            "time-stamp",
            "cov-subscription",
            "value-source",
            "scale",
            "prescale",
            "address-binding",
            "shed-level",
            "authentication-factor",
            "event-parameter",
        ]
    );
}
