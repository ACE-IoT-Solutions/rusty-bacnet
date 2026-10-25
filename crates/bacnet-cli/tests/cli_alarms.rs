//! Real CLI processes reading a device's alarms decoded (#2077):
//! `alarms` (GetEventInformation, paged), `alarm-summary` and
//! `enrollment-summary`, as tables and as JSON.
#[allow(dead_code)]
mod support;

use std::borrow::Cow;
use std::net::Ipv4Addr;

use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::{BACnetServer, GetEventInformationBudget};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, NotifyType, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};

/// An event-initiating object with a fixed event state, acknowledged
/// transitions and timestamps, so the output doesn't depend on the clock.
struct FixedAlarm {
    oid: ObjectIdentifier,
    name: String,
    state: EventState,
    /// Acked_Transitions, first bit first.
    acked: u8,
    notify: NotifyType,
}

impl FixedAlarm {
    fn new(instance: u32, state: EventState, acked: u8, notify: NotifyType) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            name: format!("Alarm {instance}"),
            state,
            acked,
            notify,
        }
    }
}

/// `[0] Time 14:30:05` and the like, context tagged as Event_Time_Stamps
/// holds them: a date and time, a sequence number, and a time.
fn timestamps() -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::ApplicationData(vec![0x2E, 0xA4, 126, 10, 25, 7, 0xB4, 14, 30, 5, 0, 0x2F]),
        PropertyValue::ApplicationData(vec![0x19, 42]),
        PropertyValue::ApplicationData(vec![0x0C, 7, 0, 0, 0]),
    ])
}

impl BACnetObject for FixedAlarm {
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
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(self.oid),
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString(self.name.clone()),
            PropertyIdentifier::OBJECT_TYPE => {
                PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw())
            }
            PropertyIdentifier::EVENT_STATE => PropertyValue::Enumerated(self.state.to_raw()),
            PropertyIdentifier::ACKED_TRANSITIONS => PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.acked << 5],
            },
            PropertyIdentifier::EVENT_TIME_STAMPS => timestamps(),
            PropertyIdentifier::NOTIFY_TYPE => PropertyValue::Enumerated(self.notify.to_raw()),
            PropertyIdentifier::EVENT_ENABLE => PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xE0],
            },
            PropertyIdentifier::NOTIFICATION_CLASS => PropertyValue::Unsigned(5),
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                ));
            }
        })
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
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ])
    }
}

/// The lab device with three fixed alarms (one an event, not an alarm)
/// and AI:1's fault cleared, answering GetEventInformation `page` objects
/// at a time.
async fn server(page: usize) -> (BACnetServer<BipTransport>, String) {
    let mut db = lab_database(1234);
    db.get_mut(&ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap())
        .unwrap()
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
            None,
        )
        .unwrap();
    // The lab's intrinsic reporting objects name class 0.
    db.add(Box::new(NotificationClass::new(0, "Default").unwrap()))
        .unwrap();
    let mut class = NotificationClass::new(5, "Plant").unwrap();
    class.priority = [3, 5, 200];
    db.add(Box::new(class)).unwrap();
    for alarm in [
        FixedAlarm::new(1, EventState::HIGH_LIMIT, 0b011, NotifyType::ALARM),
        FixedAlarm::new(2, EventState::FAULT, 0b111, NotifyType::ALARM),
        FixedAlarm::new(3, EventState::NORMAL, 0b110, NotifyType::EVENT),
    ] {
        db.add(Box::new(alarm)).unwrap();
    }
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(db)
        .get_event_information_budget(GetEventInformationBudget {
            max_returned_summaries: page,
            ..Default::default()
        })
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

async fn json(target: &str, args: &[&str]) -> Value {
    serde_json::from_str(&cli(target, "json", args).await).unwrap()
}

fn object(instance: u32) -> Value {
    json!({"type": "analog-value", "type_number": 2, "instance": instance})
}

/// The summary `alarms` prints for FixedAlarm `instance`.
fn summary(instance: u32, state: Value, acked: Value, notify: Value) -> Value {
    json!({
        "object": object(instance),
        "event_state": state,
        "acknowledged_transitions": acked,
        "event_enable": {"bits": "111", "names": ["to-offnormal", "to-fault", "to-normal"]},
        "notify_type": notify,
        "event_timestamps": {
            "to_offnormal": {"element": "time-stamp", "text": "2026-10-25 Sun 14:30:05"},
            "to_fault": {"element": "time-stamp", "text": "seq 42"},
            "to_normal": {"element": "time-stamp", "text": "07:00"}
        },
        "event_priorities": {"to_offnormal": 3, "to_fault": 5, "to_normal": 200}
    })
}

fn all_summaries() -> Vec<Value> {
    let alarm = json!({"value": 0, "name": "alarm"});
    vec![
        summary(
            1,
            json!({"value": 3, "name": "high-limit"}),
            json!({"bits": "011", "names": ["to-fault", "to-normal"]}),
            alarm.clone(),
        ),
        summary(
            2,
            json!({"value": 1, "name": "fault"}),
            json!({"bits": "111", "names": ["to-offnormal", "to-fault", "to-normal"]}),
            alarm,
        ),
        // Normal, but its to-normal transition is unacknowledged.
        summary(
            3,
            json!({"value": 0, "name": "normal"}),
            json!({"bits": "110", "names": ["to-offnormal", "to-fault"]}),
            json!({"value": 1, "name": "event"}),
        ),
    ]
}

/// A CLI run that must fail, and its stderr.
async fn refused(target: &str, args: &[&str]) -> String {
    let mut all = vec!["--interface", "127.0.0.1", "--port", "0", args[0], target];
    all.extend(&args[1..]);
    let output = support::run(all).await;
    assert!(!output.status.success(), "{args:?} succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn alarms_are_decoded_in_json_and_the_table() {
    let (mut server, target) = server(256).await;
    assert_eq!(
        json(&target, &["alarms"]).await,
        json!({"summaries": all_summaries(), "more_events": false})
    );
    let table = cli(&target, "table", &["alarms"]).await;
    let header: Vec<&str> = table
        .lines()
        .find(|line| line.starts_with("| Object"))
        .unwrap_or_else(|| panic!("no header: {table}"))
        .trim_matches('|')
        .split(" | ")
        .map(str::trim)
        .collect();
    assert_eq!(
        header,
        [
            "Object",
            "State",
            "Acked",
            "Enabled",
            "Notify",
            "To-offnormal",
            "To-fault",
            "To-normal",
            "Priorities"
        ],
        "{table}"
    );
    // The bit names share the column border's bar, so data rows are
    // checked by their start and their cells.
    for (start, notify) in [
        (
            "| analog-value 1 | high-limit | to-fault | to-normal ",
            "| alarm ",
        ),
        (
            "| analog-value 2 | fault      | to-offnormal | to-fault | to-normal ",
            "| alarm ",
        ),
        (
            "| analog-value 3 | normal     | to-offnormal | to-fault ",
            "| event ",
        ),
    ] {
        let line = table.lines().find(|line| line.starts_with(start));
        let line = line.unwrap_or_else(|| panic!("no row {start:?}: {table}"));
        for cell in [
            notify,
            "2026-10-25 Sun 14:30:05",
            "seq 42",
            "07:00",
            "3/5/200",
        ] {
            assert!(line.contains(cell), "{cell}: {line}");
        }
    }
    assert!(
        table
            .lines()
            .any(|line| line == "3 object(s) with an active event or an unacknowledged transition"),
        "{table}"
    );
    assert!(!table.contains("raw"), "{table}");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn alarms_follow_more_events_to_the_end_or_the_cap() {
    // One object an answer: three answers, each after the last object.
    let (mut server, target) = server(1).await;
    assert_eq!(
        json(&target, &["alarms"]).await,
        json!({"summaries": all_summaries(), "more_events": false})
    );
    assert_eq!(
        json(&target, &["alarms", "--max", "2"]).await,
        json!({"summaries": all_summaries()[..2], "more_events": true})
    );
    let table = cli(&target, "table", &["alarms", "--max", "2"]).await;
    assert!(
        table.contains("2 object(s) with an active event or an unacknowledged transition"),
        "{table}"
    );
    assert!(
        table.contains("The device holds more; stopped at --max 2."),
        "{table}"
    );
    refused(&target, &["alarms", "--max", "0"]).await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn alarm_summary_lists_the_alarms_named() {
    let (mut server, target) = server(256).await;
    // The device lists them in its own order.
    let mut answer = json(&target, &["alarm-summary"]).await;
    answer
        .as_array_mut()
        .unwrap()
        .sort_by_key(|entry| entry["object"]["instance"].as_u64());
    assert_eq!(
        answer,
        json!([
            {
                "object": object(1),
                "alarm_state": {"value": 3, "name": "high-limit"},
                "acknowledged_transitions": {"bits": "011", "names": ["to-fault", "to-normal"]}
            },
            {
                "object": object(2),
                "alarm_state": {"value": 1, "name": "fault"},
                "acknowledged_transitions": {
                    "bits": "111",
                    "names": ["to-offnormal", "to-fault", "to-normal"]
                }
            }
        ])
    );
    let table = cli(&target, "table", &["alarm-summary"]).await;
    let mut lines: Vec<&str> = table.lines().collect();
    // The two rows in the device's order.
    let rows = [
        "| analog-value 1 | high-limit | to-fault | to-normal                |",
        "| analog-value 2 | fault      | to-offnormal | to-fault | to-normal |",
    ];
    if lines.get(3) == Some(&rows[1]) {
        lines.swap(3, 5);
    }
    assert_eq!(
        lines,
        [
            "+----------------+------------+-------------------------------------+",
            "| Object         | State      | Acked                               |",
            "+===================================================================+",
            rows[0],
            "|----------------+------------+-------------------------------------|",
            rows[1],
            "+----------------+------------+-------------------------------------+",
            "2 object(s) in alarm",
        ],
        "{table}"
    );
    server.stop().await.unwrap();
}

/// The objects of an `enrollment-summary` answer as (type, instance),
/// sorted: the device lists them in its own order.
fn enrolled(answer: &Value) -> Vec<(String, u64)> {
    let mut objects: Vec<(String, u64)> = answer
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["object"]["type"].as_str().unwrap().to_string(),
                entry["object"]["instance"].as_u64().unwrap(),
            )
        })
        .collect();
    objects.sort();
    objects
}

#[tokio::test]
async fn enrollment_summary_names_entries_and_sends_the_filters() {
    let (mut server, target) = server(256).await;
    let answer = json(
        &target,
        &["enrollment-summary", "--event-type", "out-of-range"],
    )
    .await;
    assert_eq!(
        enrolled(&answer),
        [
            ("analog-input".to_string(), 1),
            ("analog-input".to_string(), 2),
            ("analog-output".to_string(), 1)
        ]
    );
    assert_eq!(
        answer[0]["event_type"],
        json!({"value": 5, "name": "out-of-range"})
    );
    assert_eq!(
        answer[0]["event_state"],
        json!({"value": 0, "name": "normal"})
    );
    assert_eq!(answer[0]["priority"], json!(255));
    assert_eq!(answer[0]["notification_class"], json!(0));

    // Every lab enrollment is normal, fully acknowledged, in class 0 and at
    // priority 255, so each filter below matches all of them or none.
    let all = enrolled(&json(&target, &["enrollment-summary"]).await);
    assert_eq!(all.len(), 7, "{all:?}");
    for (flags, matches) in [
        (&["--acked", "acked"][..], true),
        (&["--acked", "not-acked"], false),
        (&["--state", "normal"], true),
        (&["--state", "active"], false),
        (&["--priority", "255"], true),
        (&["--priority", "0-254"], false),
        (&["--class", "0"], true),
        (&["--class", "5"], false),
    ] {
        let mut args = vec!["enrollment-summary"];
        args.extend(flags);
        let found = enrolled(&json(&target, &args).await);
        if matches {
            assert_eq!(found, all, "{flags:?}");
        } else {
            assert!(found.is_empty(), "{flags:?}: {found:?}");
        }
    }
    // Names in either spelling.
    let states = json(
        &target,
        &["enrollment-summary", "--event-type", "CHANGE_OF_STATE"],
    )
    .await;
    assert_eq!(enrolled(&states).len(), 3, "{states}");

    let table = cli(&target, "table", &["enrollment-summary", "--class", "5"]).await;
    assert_eq!(table.trim_end(), "No event enrollments match.");
    let table = cli(
        &target,
        "table",
        &["enrollment-summary", "--event-type", "buffer-ready"],
    )
    .await;
    assert!(
        table.contains("| trend-log 1 | buffer-ready | normal | 255      | 0                  |"),
        "{table}"
    );
    assert!(table.contains("1 enrollment(s)"), "{table}");

    let stderr = refused(&target, &["enrollment-summary", "--event-type", "bogus"]).await;
    assert!(stderr.contains("unknown event type 'bogus'"), "{stderr}");
    let stderr = refused(&target, &["enrollment-summary", "--priority", "9-3"]).await;
    assert!(stderr.contains("runs backward"), "{stderr}");
    server.stop().await.unwrap();
}
