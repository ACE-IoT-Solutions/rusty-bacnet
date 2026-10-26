//! Real CLI processes reading the lab device's values through the shared
//! formatter (#2078): enumerations and bits by name, constructed values
//! rendered, and in JSON the text beside its typed form. Binary and
//! multi-state states carry their objects' state texts (#2132).
#[allow(dead_code)]
mod support;

use std::borrow::Cow;
use std::net::Ipv4Addr;

use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDestination, BACnetRecipient, BACnetTimeValue, BACnetWeekNDay,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};

fn hm(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

/// A binary or multi-state object that has a state but refuses every read
/// of its state texts.
struct Textless {
    oid: ObjectIdentifier,
    name: String,
    state: PropertyValue,
}

impl BACnetObject for Textless {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(self.oid.object_type().to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(self.state.clone()),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
        ])
    }
}

/// Multi-State Value 2 at state 2, its State_Text `Off`, `Low` (behind a
/// right-to-left override) and `High`; Binary Value 2, inactive, its texts
/// `Running` and `Stopped`; and Multi-State Value 3 at state 2 and Binary
/// Value 3 active, which refuse their texts.
fn state_objects() -> Vec<Box<dyn BACnetObject>> {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    let mut speed = MultiStateValueObject::new(2, "Fan Speed", 3).unwrap();
    speed
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            None,
            PropertyValue::List(vec![text("Off"), text("\u{202E}Low"), text("High")]),
            None,
        )
        .unwrap();
    speed.set_relinquish_default(2).unwrap();
    let mut fan = BinaryValueObject::new(2, "Fan").unwrap();
    for (property, value) in [
        (PropertyIdentifier::ACTIVE_TEXT, "Running"),
        (PropertyIdentifier::INACTIVE_TEXT, "Stopped"),
    ] {
        fan.write_property(property, None, text(value), None)
            .unwrap();
    }
    let textless = |object_type, state| {
        Box::new(Textless {
            oid: ObjectIdentifier::new(object_type, 3).unwrap(),
            name: format!("Textless {object_type}"),
            state,
        }) as Box<dyn BACnetObject>
    };
    vec![
        Box::new(speed),
        Box::new(fan),
        textless(ObjectType::MULTI_STATE_VALUE, PropertyValue::Unsigned(2)),
        textless(ObjectType::BINARY_VALUE, PropertyValue::Enumerated(1)),
    ]
}

/// The lab device, with a Schedule, a Calendar, a Notification Class that
/// has a recipient, and the [`state_objects`].
async fn server() -> (BACnetServer<BipTransport>, String) {
    let mut db = lab_database(1234);
    for object in state_objects() {
        db.add(object).unwrap();
    }
    let mut schedule = ScheduleObject::new(1, "Occupancy", PropertyValue::Real(60.0)).unwrap();
    schedule
        .set_weekly_schedule(
            0,
            vec![
                BACnetTimeValue {
                    time: hm(7, 0),
                    value: PropertyValue::Real(72.5),
                },
                BACnetTimeValue {
                    time: hm(18, 0),
                    value: PropertyValue::Null,
                },
            ],
        )
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
    let mut calendar = CalendarObject::new(1, "Holidays").unwrap();
    calendar
        .add_date_entry(BACnetCalendarEntry::Date(Date {
            year: 126,
            month: 12,
            day: 25,
            day_of_week: Date::UNSPECIFIED,
        }))
        .unwrap();
    calendar
        .add_date_entry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: 0xFF,
            week_of_month: 0xFF,
            day_of_week: 1,
        }))
        .unwrap();
    db.add(Box::new(calendar)).unwrap();
    let mut alarms = NotificationClass::new(2, "Desk").unwrap();
    alarms
        .add_destination(BACnetDestination {
            valid_days: DaysOfWeek::from_bits_truncate(0b001_1111),
            from_time: hm(8, 0),
            to_time: hm(17, 0),
            recipient: BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap(),
            ),
            process_identifier: 7,
            issue_confirmed_notifications: true,
            transitions: EventTransitionBits::all(),
        })
        .unwrap();
    db.add(Box::new(alarms)).unwrap();
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(db)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (server, target)
}

async fn cli(target: &str, format: &str, args: &[&str]) -> String {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
    ];
    all.push(args[0]);
    all.push(target);
    all.extend(&args[1..]);
    let output = support::run(all).await;
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `read` of one property, as JSON: its text value and its typed form.
async fn read(target: &str, object: &str, property: &str) -> (String, Value) {
    let output = cli(target, "json", &["read", object, property]).await;
    let json: Value = serde_json::from_str(&output).unwrap();
    (
        json["value"].as_str().unwrap().to_string(),
        json["decoded"].clone(),
    )
}

#[tokio::test]
async fn reads_name_enumerations_bits_and_constructed_values() {
    let (mut server, target) = server().await;
    let cases: Vec<(&str, &str, &str, Value)> = vec![
        (
            "ai:1",
            "reliability",
            "open-loop",
            json!({"value": 4, "name": "open-loop"}),
        ),
        (
            "ai:1",
            "units",
            "degrees-fahrenheit",
            json!({"value": 64, "name": "degrees-fahrenheit"}),
        ),
        (
            "ai:1",
            "status-flags",
            "in-alarm | fault | out-of-service",
            json!({"bits": "1101", "names": ["in-alarm", "fault", "out-of-service"]}),
        ),
        (
            "ai:1",
            "object-identifier",
            "analog-input 1",
            json!({"type": "analog-input", "type_number": 0, "instance": 1}),
        ),
        // The state with its object's text; the typed form unchanged.
        (
            "bi:1",
            "present-value",
            "inactive (Inactive)",
            json!({"value": 0, "name": "inactive"}),
        ),
        (
            "schedule:1",
            "weekly-schedule",
            "[Mon 07:00 = 72.5, 18:00 = null; Tue none; Wed none; Thu none; Fri none; \
             Sat none; Sun none]",
            json!([
                {"element": "daily-schedule", "text": "Mon 07:00 = 72.5, 18:00 = null"},
                {"element": "daily-schedule", "text": "Tue none"},
                {"element": "daily-schedule", "text": "Wed none"},
                {"element": "daily-schedule", "text": "Thu none"},
                {"element": "daily-schedule", "text": "Fri none"},
                {"element": "daily-schedule", "text": "Sat none"},
                {"element": "daily-schedule", "text": "Sun none"}
            ]),
        ),
        (
            "schedule:1",
            "weekly-schedule[1]",
            "Mon 07:00 = 72.5, 18:00 = null",
            json!({"element": "daily-schedule", "text": "Mon 07:00 = 72.5, 18:00 = null"}),
        ),
        // An indexed day is named for its place in the week.
        (
            "schedule:1",
            "weekly-schedule[3]",
            "Wed none",
            json!({"element": "daily-schedule", "text": "Wed none"}),
        ),
        (
            "schedule:1",
            "weekly-schedule[7]",
            "Sun none",
            json!({"element": "daily-schedule", "text": "Sun none"}),
        ),
        (
            "calendar:1",
            "date-list",
            "[2026-12-25, Mon, any week, any month]",
            json!([{"element": "calendar-entry", "text": "2026-12-25"}, {"element": "calendar-entry", "text": "Mon, any week, any month"}]),
        ),
        (
            "nc:2",
            "recipient-list",
            "[device 99 process 7: Mon-Fri 08:00-17:00, confirmed, \
             to-offnormal | to-fault | to-normal]",
            json!([{"element": "destination", "text": "device 99 process 7: Mon-Fri 08:00-17:00, confirmed, \
                    to-offnormal | to-fault | to-normal"}]),
        ),
    ];
    for (object, property, text, decoded) in cases {
        let (value, data) = read(&target, object, property).await;
        assert_eq!(value, text, "{object} {property}");
        assert_eq!(data, decoded, "{object} {property}");
    }

    // Every slot of a priority array, not the first few.
    let (value, data) = read(&target, "ao:1", "priority-array").await;
    let slots: Vec<&str> = value
        .trim_matches(|c| c == '[' || c == ']')
        .split(", ")
        .collect();
    assert_eq!(slots.len(), 16, "{value}");
    assert_eq!(slots[9], "55", "{value}");
    assert_eq!(data[9], json!(55.0));
    assert_eq!(data[0], Value::Null);

    // Timestamps render, in the text and as constructed elements.
    let (value, data) = read(&target, "ai:1", "event-time-stamps").await;
    assert!(!value.contains("raw"), "{value}");
    assert_eq!(data.as_array().map(Vec::len), Some(3), "{data}");
    assert!(data[0]["text"].is_string(), "{data}");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn tables_and_readm_use_the_same_names() {
    let (mut server, target) = server().await;
    let table = cli(&target, "table", &["read", "ai:1", "reliability"]).await;
    assert!(table.contains("open-loop"), "{table}");
    let table = cli(
        &target,
        "table",
        &["readm", "ai:1", "event-state,status-flags"],
    )
    .await;
    assert!(table.contains("| fault "), "{table}");
    assert!(
        table.contains("in-alarm | fault | out-of-service"),
        "{table}"
    );
    assert!(!table.contains("enumerated("), "{table}");
    assert!(!table.contains("bits("), "{table}");

    let json: Value = serde_json::from_str(
        &cli(
            &target,
            "json",
            &["readm", "ai:2", "event-state,limit-enable,profile-name"],
        )
        .await,
    )
    .unwrap();
    assert_eq!(json[0]["value"], "normal");
    assert_eq!(json[0]["decoded"], json!({"value": 0, "name": "normal"}));
    // Named bits with none set: `none`, and an empty list of names.
    assert_eq!(json[1]["value"], "none");
    assert_eq!(json[1]["decoded"], json!({"bits": "00", "names": []}));
    // A property the object lacks has an error and no typed form.
    assert!(
        json[2]["value"].as_str().unwrap().starts_with("ERROR"),
        "{json}"
    );
    assert!(json[2].get("decoded").is_none(), "{json}");
    server.stop().await.unwrap();
}

/// A multi-state and a binary value's states print with their objects'
/// texts, cleaned, in JSON beside the unchanged typed form and in the
/// table; an object that refuses its texts prints today's plain states.
/// A list read names states too, with texts it read itself or one more
/// request, and an array's size is never a state (#2132).
#[tokio::test]
async fn states_print_with_their_objects_state_texts() {
    let (mut server, target) = server().await;
    for (object, property, text, decoded) in [
        ("msv:2", "present-value", "2 (Low)", json!(2)),
        ("msv:2", "relinquish-default", "2 (Low)", json!(2)),
        ("msv:2", "priority-array[0]", "16", json!(16)),
        (
            "bv:2",
            "present-value",
            "inactive (Stopped)",
            json!({"value": 0, "name": "inactive"}),
        ),
        // Refused: the plain state.
        ("msv:3", "present-value", "2", json!(2)),
        (
            "bv:3",
            "present-value",
            "active",
            json!({"value": 1, "name": "active"}),
        ),
    ] {
        let (value, data) = read(&target, object, property).await;
        assert_eq!(value, text, "{object} {property}");
        assert_eq!(data, decoded, "{object} {property}");
    }
    let table = cli(&target, "table", &["read", "msv:2", "pv"]).await;
    assert!(table.contains("2 (Low)"), "{table}");

    // The refused object first: its refusal doesn't stop the others' reads.
    let json: Value = serde_json::from_str(
        &cli(
            &target,
            "json",
            &[
                "readm",
                "msv:3",
                "pv",
                "msv:2",
                "pv,state-text",
                "bv:2",
                "pv",
            ],
        )
        .await,
    )
    .unwrap();
    let values: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["value"].as_str().unwrap())
        .collect();
    assert_eq!(values[..2], ["2", "2 (Low)"], "{json}");
    assert_eq!(values[3], "inactive (Stopped)", "{json}");

    let all: Value =
        serde_json::from_str(&cli(&target, "json", &["read", "bv:2", "all"]).await).unwrap();
    let present = all
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["property"] == "PRESENT_VALUE")
        .unwrap_or_else(|| panic!("{all}"));
    assert_eq!(present["value"], "inactive (Stopped)", "{all}");
    server.stop().await.unwrap();
}
