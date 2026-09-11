use super::*;

// ── Real-object PICS writability/createability tests ───────────────────────
//
// The tests above use minimal stub objects. The tests below use the real
// bacnet-objects implementations to verify that PICS writable flags and
// createability match the objects' own `write_property` arms and the runtime
// `handle_create_object` factory. This is the core regression guard for the
// shared-truth-source fix (issue #115): PICS and runtime dispatch must agree.

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};

/// Build a database with one of each of the 9 core I/O/V object types.
fn make_real_objects_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AnalogInputObject::new(1, "ai-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(AnalogOutputObject::new(1, "ao-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(AnalogValueObject::new(1, "av-1", 95).unwrap()))
        .unwrap();
    db.add(Box::new(BinaryInputObject::new(1, "bi-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryOutputObject::new(1, "bo-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "bv-1").unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateInputObject::new(1, "msi-1", 2).unwrap()))
        .unwrap();
    db.add(Box::new(
        MultiStateOutputObject::new(1, "mso-1", 2).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "msv-1", 2).unwrap()))
        .unwrap();
    db
}

/// Helper: look up a property's writable flag in a PICS ObjectTypeSupport.
fn pics_writable<'a>(pics: &'a Pics, object_type: ObjectType, pid: PropertyIdentifier) -> bool {
    pics.supported_object_types
        .iter()
        .find(|ot| ot.object_type == object_type)
        .and_then(|ot| {
            ot.supported_properties
                .iter()
                .find(|p| p.property_id == pid)
        })
        .map(|p| p.access.writable)
        .expect("property should be in the PICS list")
}

#[test]
fn pics_event_properties_writable_on_analog_types() {
    // The old heuristic omitted LIMIT_ENABLE, NOTIFY_TYPE, TIME_DELAY,
    // EVENT_ENABLE — which the objects actually accept via write_generic_event_properties!.
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
    ] {
        for pid in [
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ] {
            assert!(
                pics_writable(&pics, ot, pid),
                "{ot:?}: {pid:?} should be writable (accepted by write_generic_event_properties!)"
            );
        }
    }
}

/// Before #229 the four types below had no write path for the event set at all
/// — Time_Delay and Notify_Type were absent entirely — so PICS advertised what
/// no client could commission and no notification could ever be distributed.
#[test]
fn pics_event_properties_writable_on_binary_and_multistate_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    for ot in [
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        for pid in [
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ] {
            assert!(
                pics_writable(&pics, ot, pid),
                "{ot:?}: {pid:?} should be writable (accepted by write_generic_event_properties!)"
            );
        }
        // Denied by the same macro, so PICS must not advertise it writable.
        assert!(
            !pics_writable(&pics, ot, PropertyIdentifier::ACKED_TRANSITIONS),
            "{ot:?}: ACKED_TRANSITIONS must stay read-only"
        );
    }
}

#[test]
fn pics_priority_array_writable_on_commandable_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // Commandable types accept PRIORITY_ARRAY (direct) and PRESENT_VALUE
    // (via the priority array). RELINQUISH_DEFAULT grew a validated write arm
    // in #270 (the standard permits writability), so the PICS advertises it.
    for ot in [
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRIORITY_ARRAY),
            "{ot:?}: PRIORITY_ARRAY should be writable"
        );
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRESENT_VALUE),
            "{ot:?}: PRESENT_VALUE should be writable"
        );
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::RELINQUISH_DEFAULT),
            "{ot:?}: RELINQUISH_DEFAULT should be writable (#270)"
        );
    }
}

#[test]
fn pics_input_present_value_writable_only_when_out_of_service() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // Input types (AI, BI, MSI) accept PRESENT_VALUE writes when
    // out-of-service. PICS reports the type-level writability (the runtime
    // enforces the out-of-service guard), so PRESENT_VALUE is writable.
    // This was a false-negative in the old heuristic for AI (AI was excluded);
    // the trait override now mirrors the real write_property arm.
    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::BINARY_INPUT,
        ObjectType::MULTI_STATE_INPUT,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::PRESENT_VALUE),
            "{ot:?}: PRESENT_VALUE should be writable (accepted when out-of-service)"
        );
    }
}

#[test]
fn pics_state_text_writable_on_multistate_types() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // All three multistate types accept STATE_TEXT writes (array-indexed).
    for ot in [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        assert!(
            pics_writable(&pics, ot, PropertyIdentifier::STATE_TEXT),
            "{ot:?}: STATE_TEXT should be writable"
        );
    }
}

#[test]
fn pics_fixed_readonly_properties_never_writable() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // The universal identifiers below are never writable. Number_Of_States is
    // likewise fixed on every object type that exposes it.
    for ot in pics.supported_object_types.iter() {
        for pid in [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::NUMBER_OF_STATES,
        ] {
            if ot.supported_properties.iter().any(|p| p.property_id == pid) {
                assert!(
                    !pics_writable(&pics, ot.object_type, pid),
                    "{ot:?}: {pid:?} must never be writable"
                );
            }
        }
    }
}

#[test]
fn pics_createability_matches_runtime_factory() {
    let db = make_real_objects_db();
    let pics = generate_pics(&db, &ServerConfig::default(), &make_pics_config());

    // The 8 types handle_create_object actually constructs must be createable.
    for ot in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        let entry = pics
            .supported_object_types
            .iter()
            .find(|e| e.object_type == ot)
            .expect("type should be in PICS");
        assert!(
            entry.createable,
            "{ot:?} should be createable (factory constructs it)"
        );
    }

    // AnalogValue is NOT in the factory — PICS must not advertise createability.
    let av = pics
        .supported_object_types
        .iter()
        .find(|e| e.object_type == ObjectType::ANALOG_VALUE)
        .expect("ANALOG_VALUE should be in PICS");
    assert!(
        !av.createable,
        "AnalogValue must NOT be createable (factory rejects it with UNSUPPORTED_OBJECT_TYPE)"
    );
}

#[test]
fn pics_writability_matches_runtime_write_property() {
    // Cross-check: PICS reports LIMIT_ENABLE writable on AnalogInput AND
    // write_property actually accepts it. The old heuristic reported it
    // non-writable (false-negative); the trait override fixes both.
    use bacnet_objects::event::LimitEnable;
    use bacnet_objects::traits::BACnetObject;

    let mut ai = AnalogInputObject::new(1, "ai-1", 95).unwrap();
    // PICS (via the trait method) must report it writable.
    assert!(
        ai.is_writable_property(PropertyIdentifier::LIMIT_ENABLE),
        "is_writable_property must report LIMIT_ENABLE writable on AnalogInput"
    );
    // And the runtime write_property must accept it.
    let bits = LimitEnable::BOTH.to_bits();
    let result = ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![bits],
        },
        None,
    );
    assert!(
        result.is_ok(),
        "write_property must accept LIMIT_ENABLE, got: {result:?}"
    );

    // Cross-check a property PICS reports non-writable: STATUS_FLAGS.
    assert!(
        !ai.is_writable_property(PropertyIdentifier::STATUS_FLAGS),
        "STATUS_FLAGS must not be writable on AnalogInput"
    );
    let result = ai.write_property(
        PropertyIdentifier::STATUS_FLAGS,
        None,
        PropertyValue::Boolean(true),
        None,
    );
    assert!(
        result.is_err(),
        "write_property must reject STATUS_FLAGS, got: {result:?}"
    );
}

#[test]
fn executed_services_match_dispatch_table() {
    // The three-way truth chain for #192: dispatch arms (requests/mod.rs +
    // unconfirmed.rs choice consts) -> BACnetServicesSupported bits ->
    // device::EXECUTED_SERVICES, which feeds both the Device object's
    // Protocol_Services_Supported and the PICS executor column. If a dispatch
    // arm is added or removed without updating the choice const or the device
    // constant, this test fails.
    use bacnet_types::enums::{ServiceSupported, UnconfirmedServiceChoice};

    assert!(
        !crate::server::EXECUTED_UNCONFIRMED.contains(&UnconfirmedServiceChoice::WRITE_GROUP),
        "inbound WriteGroup has no execution path"
    );

    let mut from_dispatch: Vec<u8> = crate::server::EXECUTED_CONFIRMED
        .iter()
        .map(|c| {
            ServiceSupported::from_confirmed_choice(*c)
                .expect("dispatched confirmed choice has a defined bit")
                .to_raw()
        })
        .chain(crate::server::EXECUTED_UNCONFIRMED.iter().map(|c| {
            ServiceSupported::from_unconfirmed_choice(*c)
                .expect("dispatched unconfirmed choice has a defined bit")
                .to_raw()
        }))
        .collect();
    from_dispatch.sort_unstable();

    let mut declared: Vec<u8> = bacnet_objects::device::EXECUTED_SERVICES
        .iter()
        .map(|s| s.to_raw())
        .collect();
    declared.sort_unstable();

    assert_eq!(
        declared, from_dispatch,
        "device::EXECUTED_SERVICES must equal the dispatch table's executed set"
    );
}
