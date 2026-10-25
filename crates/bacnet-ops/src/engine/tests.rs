//! The engine against an in-process lab device over a loopback link.

use std::sync::Arc;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_client::log_reader::LogCursor;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_range::LogRecords;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use super::*;
use crate::format::{Floats, StateTexts, ValueFormat, decode_all};

/// The lab device's instance.
const LAB: u32 = 1001;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A trend log holding `records` REAL samples, one a minute from midnight.
fn trend(records: u8) -> TrendLogObject {
    let mut log = TrendLogObject::new(1, "Zone Temp Log", 100).unwrap();
    for minute in 0..records {
        log.add_record(BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 25,
                day_of_week: 7,
            },
            time: Time {
                hour: 0,
                minute,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::RealValue(f32::from(minute)),
            status_flags: None,
        })
        .unwrap();
    }
    log
}

/// The lab: a device with an analog input, an analog output and a trend
/// log, an engine over a client paired with it, and the server to stop.
async fn lab() -> (Engine<LoopbackTransport>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let mut ai = AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let ao = AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(ai),
        Box::new(ao),
        Box::new(trend(3)),
        Box::new(NotificationClass::new(0, "Alarms").unwrap()),
    ];
    let database = build_database_with_extra(&identity, objects).unwrap();
    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(server_port)
        .build()
        .await
        .unwrap();
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, client_port).await.unwrap();
    client.add_device(LAB, &[2]).await.unwrap();
    let engine = Engine::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    (engine, server)
}

async fn stop(engine: Engine<LoopbackTransport>, mut server: BACnetServer<LoopbackTransport>) {
    let Ok(mut client) = Arc::try_unwrap(engine.client) else {
        panic!("the engine holds the only client");
    };
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A property read gives each property's raw value by RPM, and the shared
/// formatter shows it.
#[tokio::test(start_paused = true)]
async fn reads_properties_by_rpm_and_formats_them() {
    let (engine, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let read = engine
        .read(
            LAB,
            ai,
            Wanted::List(&[PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::UNITS]),
        )
        .await
        .unwrap();
    assert_eq!(read.object, ai);
    assert_eq!(read.via, ReadVia::Rpm);
    let value = read.rows[0].1.as_ref().unwrap();
    assert_eq!(
        decode_all(value).unwrap(),
        [PropertyValue::Real(21.5)],
        "{read:?}"
    );
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::ANALOG_INPUT, &texts, Floats::Display);
    let units = crate::format::units(&read.rows);
    assert_eq!(
        format.with_units(PropertyIdentifier::PRESENT_VALUE, value, units),
        "21.5 °C"
    );
    // One element of an array, by ReadProperty.
    let length = engine
        .read_property(
            LAB,
            oid(ObjectType::ANALOG_OUTPUT, 1),
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
        )
        .await
        .unwrap();
    assert_eq!(decode_all(&length).unwrap(), [PropertyValue::Unsigned(16)]);
    assert!(!engine.refused_rpm(LAB));
    stop(engine, server).await;
}

/// The object list and the device summary come from the Device object.
#[tokio::test(start_paused = true)]
async fn lists_objects_and_describes_the_device() {
    let (engine, server) = lab().await;
    let list = engine.list_objects(LAB).await.unwrap();
    assert!(!list.indexed);
    assert_eq!(list.stopped, None);
    for object in [
        oid(ObjectType::DEVICE, LAB),
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::ANALOG_OUTPUT, 1),
        oid(ObjectType::TREND_LOG, 1),
    ] {
        assert!(list.objects.contains(&object), "{object:?} in {list:?}");
    }
    let summary = engine.describe_device(LAB).await.unwrap();
    assert_eq!(summary.device, LAB);
    assert_eq!(summary.rows.len(), DEVICE_PROPERTIES.len());
    let properties: Vec<_> = summary.rows.iter().map(|(p, _)| *p).collect();
    assert_eq!(properties, DEVICE_PROPERTIES);
    assert!(summary.name().is_some(), "{summary:?}");
    stop(engine, server).await;
}

/// A trend log reads page by page from the oldest record, and alarms
/// decode (the lab has none).
#[tokio::test(start_paused = true)]
async fn reads_trend_pages_and_alarms() {
    let (engine, server) = lab().await;
    let log = oid(ObjectType::TREND_LOG, 1);
    let first = engine
        .read_trend(LAB, log, LogCursor::Oldest, 2)
        .await
        .unwrap();
    let LogRecords::TrendLog(records) = &first.records else {
        panic!("trend records: {first:?}");
    };
    assert_eq!(records.len(), 2);
    assert!(!first.done);
    let second = engine.read_trend(LAB, log, first.next, 2).await.unwrap();
    let LogRecords::TrendLog(records) = &second.records else {
        panic!("trend records: {second:?}");
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].log_datum, LogDatum::RealValue(2.0));
    assert!(second.done);

    let alarms = engine.alarms(LAB, None).await.unwrap();
    assert!(alarms.list_of_event_summaries.is_empty());
    assert!(!alarms.more_events);
    stop(engine, server).await;
}

/// A device that isn't in the discovery table fails the read.
#[tokio::test(start_paused = true)]
async fn an_unknown_device_fails() {
    let (engine, server) = lab().await;
    let error = engine.describe_device(4242).await.unwrap_err();
    assert!(error.to_string().contains("4242"), "{error}");
    stop(engine, server).await;
}

/// Once a reader of the engine meets an RPM refusal, every later read of
/// that device goes by ReadProperty, with the same values; other devices
/// keep RPM.
#[tokio::test(start_paused = true)]
async fn a_refusal_of_rpm_is_remembered_per_device() {
    let (engine, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let wanted = [PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::UNITS];
    let by_rpm = engine.read(LAB, ai, Wanted::List(&wanted)).await.unwrap();
    // What a reader records when the device refuses RPM.
    let mut refused = engine.reader(LAB);
    refused.rpm = false;
    engine.settle(&refused);
    assert!(engine.refused_rpm(LAB));
    assert!(!engine.refused_rpm(LAB + 1));
    let by_rp = engine.read(LAB, ai, Wanted::List(&wanted)).await.unwrap();
    assert_eq!(by_rp.via, ReadVia::Rp);
    assert_eq!(by_rp.rows, by_rpm.rows);
    stop(engine, server).await;
}

/// Every call waits for the device's gate: while another request to the
/// device holds it, nothing goes, and once it is free the call completes.
#[tokio::test(start_paused = true)]
async fn every_call_waits_for_the_device_gate() {
    let (engine, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let log = oid(ObjectType::TREND_LOG, 1);
    let wait = std::time::Duration::from_secs(30);
    let held = engine.limits().gate(LAB).acquire_owned().await.unwrap();
    let pv = [PropertyIdentifier::PRESENT_VALUE];
    assert!(
        tokio::time::timeout(wait, engine.read(LAB, ai, Wanted::List(&pv)))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(wait, engine.list_objects(LAB))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(wait, engine.read_trend(LAB, log, LogCursor::Oldest, 2))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(wait, engine.alarms(LAB, None))
            .await
            .is_err()
    );
    drop(held);
    tokio::time::timeout(wait, engine.alarms(LAB, None))
        .await
        .expect("the gate is free")
        .unwrap();
    stop(engine, server).await;
}
