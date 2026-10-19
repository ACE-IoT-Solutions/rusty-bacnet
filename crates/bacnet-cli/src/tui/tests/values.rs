//! Values as the browser and the watch list show them (#1888), against a
//! device with the objects whose values need more than an application tag
//! to read: an analog input in alarm with its event timestamps, a binary
//! and a multi-state output with state texts, a schedule with a weekly
//! schedule, and a trend log with its monitored reference.

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateOutputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetTimeValue};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::browser::{open_device, open_object};
use super::lab::lab_identity;
use super::site::{ClientLimits, Server, Site};
use super::*;
use crate::tui::app::watch::Screen;
use crate::tui::watched::Source;

const PLANT: u32 = 1001;

type P = PropertyIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn set(object: &mut dyn BACnetObject, property: P, value: PropertyValue) {
    object
        .write_property(property, None, value, None)
        .unwrap_or_else(|error| panic!("{property}: {error}"));
}

fn texts(texts: &[&str]) -> PropertyValue {
    PropertyValue::List(
        texts
            .iter()
            .map(|text| PropertyValue::CharacterString((*text).into()))
            .collect(),
    )
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

/// The plant device: Device, AI:3 above its high limit, BO:1 on at
/// priority 8, MSO:1 at state 2 from priority 12, NC:1, SCHED:1 and TL:1
/// logging AI:3.
fn plant() -> ObjectDatabase {
    let operator = CommandOrigin::Local {
        owner_device: oid(ObjectType::DEVICE, PLANT),
        initiating_object: None,
    };
    let mut ai3 =
        AnalogInputObject::new(3, "Discharge Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai3.set_present_value(84.781_75);
    set(&mut ai3, P::HIGH_LIMIT, PropertyValue::Real(80.0));
    set(&mut ai3, P::LOW_LIMIT, PropertyValue::Real(45.0));
    set(
        &mut ai3,
        P::LIMIT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0b1100_0000],
        },
    );
    set(
        &mut ai3,
        P::EVENT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        },
    );
    set(&mut ai3, P::NOTIFICATION_CLASS, PropertyValue::Unsigned(1));

    let mut bo1 = BinaryOutputObject::new(1, "Supply Fan").unwrap();
    set(
        &mut bo1,
        P::ACTIVE_TEXT,
        PropertyValue::CharacterString("On".into()),
    );
    set(
        &mut bo1,
        P::INACTIVE_TEXT,
        PropertyValue::CharacterString("Off".into()),
    );
    bo1.write_property_from(
        P::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
        &operator,
    )
    .expect("command BO:1 at priority 8");

    let mut mso1 = MultiStateOutputObject::new(1, "Fan Speed", 3).unwrap();
    set(&mut mso1, P::STATE_TEXT, texts(&["Low", "Medium", "High"]));
    mso1.write_property_from(
        P::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(2),
        Some(12),
        &operator,
    )
    .expect("command MSO:1 at priority 12");

    let mut schedule =
        ScheduleObject::new(1, "Occupancy Schedule", PropertyValue::Enumerated(0)).unwrap();
    for day in 0..5 {
        schedule
            .set_weekly_schedule(
                day,
                vec![
                    BACnetTimeValue {
                        time: at(7, 0),
                        value: PropertyValue::Enumerated(1),
                    },
                    BACnetTimeValue {
                        time: at(18, 0),
                        value: PropertyValue::Enumerated(0),
                    },
                ],
            )
            .unwrap();
    }

    let mut log = TrendLogObject::new(1, "Discharge Temp Log", 100).unwrap();
    log.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::ANALOG_INPUT, 3),
        P::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();

    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(ai3),
        Box::new(bo1),
        Box::new(mso1),
        Box::new(NotificationClass::new(1, "Alarms").unwrap()),
        Box::new(schedule),
        Box::new(log),
    ];
    let mut device = DeviceObject::new(DeviceConfig {
        identity: lab_identity(PLANT),
        vendor_name: "Rusty Lab".into(),
        model_name: "Bench-1".into(),
        ..Default::default()
    })
    .unwrap();
    let mut list = vec![device.object_identifier()];
    list.extend(objects.iter().map(|object| object.object_identifier()));
    device.set_object_list(list);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    for object in objects {
        db.add(object).unwrap();
    }
    db
}

async fn plant_site() -> Site {
    let server = Server {
        database: Some(plant()),
        ..Server::bare(100, PLANT)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    // Long enough for the server to evaluate AI:3's limits.
    site.run_for(std::time::Duration::from_secs(3)).await;
    open_device(&mut site).await;
    site
}

/// `text` with each line holding `needle` replaced by a marker: for a
/// value the server stamps with its wall clock.
pub(super) fn mask_lines(text: &str, needle: &str) -> String {
    text.lines()
        .map(|line| {
            if line.contains(needle) {
                format!("<the {needle} line, masked>")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// No property of any of these objects shows as raw octets.
fn assert_decoded(text: &str) {
    assert!(!text.contains("[raw"), "{text}");
    assert!(!text.contains("application-data"), "{text}");
}

/// Each object's properties, decoded, as the browser shows them.
#[tokio::test(start_paused = true)]
async fn the_browser_shows_constructed_and_state_values_decoded() {
    let mut site = plant_site().await;
    for (filter, name) in [
        ("analog-input 3", "ai3"),
        ("binary-output 1", "bo1"),
        ("multi-state-output 1", "mso1"),
        ("schedule 1", "schedule"),
        ("trend-log 1", "trendlog"),
    ] {
        let view = open_object(&mut site, filter).await;
        // AI:3's to-offnormal stamp is the server's wall clock, so its line
        // is masked; the stamp itself is checked below.
        let mut text = screen(&mut site.app, 120, 40);
        if name == "ai3" {
            text = mask_lines(&text, "event-time-stamps");
        }
        assert_snapshot!(format!("values_{name}_120x40"), text);
        assert_decoded(&text);
        let value = |property| view.value(property).map(str::to_string);
        match name {
            "ai3" => {
                assert_eq!(value(P::PRESENT_VALUE).as_deref(), Some("84.7818 °F"));
                assert_eq!(
                    value(P::EVENT_ALGORITHM_INHIBIT_REF).as_deref(),
                    Some("none")
                );
                let stamps = value(P::EVENT_TIME_STAMPS).unwrap();
                let first = stamps
                    .strip_prefix('[')
                    .and_then(|rest| rest.strip_suffix(", seq 0, seq 0]"))
                    .unwrap_or_else(|| panic!("{stamps}"));
                let (date, time) = first.split_once(' ').unwrap_or_else(|| panic!("{stamps}"));
                let date = date.as_bytes();
                assert!(
                    date.len() == 10 && date[4] == b'-' && date[7] == b'-' && time.contains(':'),
                    "a date and a time: {stamps}"
                );
            }
            "bo1" => assert_eq!(value(P::PRESENT_VALUE).as_deref(), Some("active (On)")),
            "mso1" => assert_eq!(value(P::PRESENT_VALUE).as_deref(), Some("2 (Medium)")),
            "trendlog" => assert_eq!(
                value(P::LOG_DEVICE_OBJECT_PROPERTY).as_deref(),
                Some("analog-input 3 present-value")
            ),
            _ => {}
        }
    }
    site.stop().await;
}

/// Type `line` on the command bar and run it.
async fn command(site: &mut Site, line: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
}

/// The watch list rounds and names values as the browser does.
#[tokio::test(start_paused = true)]
async fn the_watch_list_shows_values_as_the_browser_does() {
    let mut site = plant_site().await;
    let browsed = open_object(&mut site, "analog-input 3").await;
    for target in ["ai:3", "bo:1", "mso:1"] {
        command(&mut site, &format!("watch {PLANT} {target}")).await;
    }
    site.pump_until(|app| {
        app.watch.rows().len() == 3
            && app
                .watch
                .rows()
                .iter()
                .all(|row| !matches!(row.source, Source::Starting) && row.value.is_some())
    })
    .await;
    let values: Vec<_> = site
        .app
        .watch
        .rows()
        .iter()
        .map(|row| row.value.clone().unwrap().unwrap())
        .collect();
    assert_eq!(
        values,
        [
            browsed.value(P::PRESENT_VALUE).unwrap(),
            "active (On)",
            "2 (Medium)"
        ]
    );
    tick(&mut site.app, tokio::time::Instant::now().into_std());
    site.app.screen = Screen::Watch;
    let text = screen(&mut site.app, 120, 40);
    assert_snapshot!("values_watch_120x40", text);
    site.stop().await;
}
