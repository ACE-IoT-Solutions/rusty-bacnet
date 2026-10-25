//! The engine against an in-process lab device over a loopback link.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::Bytes;
use tokio::sync::mpsc;

use super::*;
use crate::format::{Floats, RowError, StateTexts, ValueFormat, decode_all};

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
    lab_on(|port| port).await
}

/// [`lab`], with the client's link wrapped by `wrap`.
async fn lab_on<C: TransportPort + 'static>(
    wrap: fn(LoopbackTransport) -> C,
) -> (Engine<C>, BACnetServer<LoopbackTransport>) {
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
    let client = BACnetClient::start(config, wrap(client_port))
        .await
        .unwrap();
    client.add_device(LAB, &[2]).await.unwrap();
    let engine = Engine::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    (engine, server)
}

async fn stop<C: TransportPort + 'static>(
    engine: Engine<C>,
    mut server: BACnetServer<LoopbackTransport>,
) {
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

/// A device that rejects ReadPropertyMultiple is read by ReadProperty
/// instead, and the engine remembers it: the next read of that device goes
/// straight to ReadProperty, with the same values. Errors stay structured
/// on both paths.
#[tokio::test(start_paused = true)]
async fn a_refusal_of_rpm_is_remembered_per_device() {
    let (engine, server) = lab_on(RefusesRpm::new).await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    // State_Text: an analog input has none.
    let wanted = [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATE_TEXT,
    ];
    assert!(!engine.refused_rpm(LAB));
    let first = engine.read(LAB, ai, Wanted::List(&wanted)).await.unwrap();
    assert_eq!(first.via, ReadVia::Rp);
    assert!(engine.refused_rpm(LAB), "the refusal is remembered");
    assert!(!engine.refused_rpm(LAB + 1));
    let rejected = engine.client.transport().rejected();
    assert_eq!(rejected, 1);
    let second = engine.read(LAB, ai, Wanted::List(&wanted)).await.unwrap();
    assert_eq!(second.via, ReadVia::Rp);
    assert_eq!(
        engine.client.transport().rejected(),
        rejected,
        "no RPM was tried again"
    );
    assert_eq!(second.rows, first.rows);
    assert_eq!(
        decode_all(second.rows[0].1.as_ref().unwrap()).unwrap(),
        [PropertyValue::Real(21.5)]
    );
    let missing = second.rows[1].1.as_ref().unwrap_err();
    assert_eq!(
        missing.class_code(),
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        "{missing:?}"
    );
    stop(engine, server).await;
}

/// An RPM answer's per-property error comes back as the device's class and
/// code, shown as `CLASS:CODE`.
#[tokio::test(start_paused = true)]
async fn a_missing_property_is_a_typed_error_row() {
    let (engine, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let wanted = [PropertyIdentifier::STATE_TEXT];
    let read = engine.read(LAB, ai, Wanted::List(&wanted)).await.unwrap();
    assert_eq!(read.via, ReadVia::Rpm);
    let error = read.rows[0].1.as_ref().unwrap_err();
    assert_eq!(
        *error,
        RowError::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_PROPERTY,
        })
    );
    assert_eq!(error.to_string(), "PROPERTY:UNKNOWN_PROPERTY");
    assert_eq!(RowError::NoValue.to_string(), "no value");
    stop(engine, server).await;
}

/// The client's side of the lab link, answering every ReadPropertyMultiple
/// itself with a Reject (unrecognized service), as a device without RPM
/// does. Everything else passes through.
struct RefusesRpm {
    inner: LoopbackTransport,
    /// Where the client's receive channel is fed from, once started.
    inbox: Mutex<Option<mpsc::Sender<ReceivedNpdu>>>,
    rejected: AtomicUsize,
}

impl RefusesRpm {
    fn new(inner: LoopbackTransport) -> Self {
        Self {
            inner,
            inbox: Mutex::new(None),
            rejected: AtomicUsize::new(0),
        }
    }

    /// ReadPropertyMultiple requests answered with a Reject so far.
    fn rejected(&self) -> usize {
        self.rejected.load(Ordering::Relaxed)
    }

    /// The invoke ID of `npdu` when it is a local, unsegmented
    /// ReadPropertyMultiple request: NPDU version and a control octet with
    /// no addresses, then a Confirmed-Request APDU.
    fn rpm_invoke_id(npdu: &[u8]) -> Option<u8> {
        match npdu {
            [0x01, control, apdu, _, invoke_id, service, ..]
                if control & 0x28 == 0 && apdu & 0xF8 == 0x00 && *service == 14 =>
            {
                Some(*invoke_id)
            }
            _ => None,
        }
    }
}

impl TransportPort for RefusesRpm {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let mut from_peer = self.inner.start().await?;
        let (tx, rx) = mpsc::channel(256);
        *self.inbox.lock().unwrap() = Some(tx.clone());
        tokio::spawn(async move {
            while let Some(npdu) = from_peer.recv().await {
                if tx.send(npdu).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.inbox.lock().unwrap().take();
        self.inner.stop().await
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let Some(invoke_id) = Self::rpm_invoke_id(npdu) else {
            return self.inner.send_unicast(npdu, mac).await;
        };
        self.rejected.fetch_add(1, Ordering::Relaxed);
        // A Reject PDU for UNRECOGNIZED_SERVICE (9), from the device.
        let reject = Bytes::from(vec![0x01, 0x00, 0x60, invoke_id, 9]);
        let inbox = self.inbox.lock().unwrap().clone();
        if let Some(inbox) = inbox {
            let source = MacAddr::from_slice(mac);
            let back = ReceivedNpdu::unverified(reject, source, false, Vec::new(), None);
            let _ = inbox.send(back).await;
        }
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }
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

/// A trend page is charged for every request it sends, not once per page:
/// the oldest page reads the log's counts before its ReadRange, and the
/// next page, by sequence number, sends the ReadRange alone.
#[tokio::test(start_paused = true)]
async fn a_trend_page_is_charged_per_request() {
    use crate::limits::{Account, charged};
    let (engine, server) = lab().await;
    let log = oid(ObjectType::TREND_LOG, 1);
    let first = Arc::new(Account::new(None));
    let page = charged(
        Arc::clone(&first),
        engine.read_trend(LAB, log, LogCursor::Oldest, 2),
    )
    .await
    .unwrap();
    assert!(
        first.requests() >= 3,
        "counts then range: {}",
        first.requests()
    );
    let second = Arc::new(Account::new(None));
    charged(
        Arc::clone(&second),
        engine.read_trend(LAB, log, page.next, 2),
    )
    .await
    .unwrap();
    assert_eq!(second.requests(), 1);
    stop(engine, server).await;
}

/// A local Who-Is puts the lab device in the table; a device instance is
/// no station address for a directed one.
#[tokio::test(start_paused = true)]
async fn a_who_is_fills_the_table() {
    let (engine, server) = lab().await;
    engine.client.clear_devices().await;
    assert!(engine.device(LAB).await.is_none());
    engine.who_is(WhoIsScope::Local, None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let found = engine.device(LAB).await.expect("the lab device");
    assert_eq!(found.vendor_id, 42);
    assert!(
        engine
            .devices()
            .await
            .iter()
            .any(|d| d.object_identifier.instance_number() == LAB)
    );
    let refused = engine
        .who_is(WhoIsScope::Directed("1234"), None)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("station address"), "{refused}");
    stop(engine, server).await;
}
