//! The hub against an in-process lab device over a loopback link. What the
//! hub holds is checked against the device's own subscription table
//! (Active_COV_Subscriptions), not only the hub's counters.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_encoding::constructed::decode_cov_subscription;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::mpsc;

use super::*;
use crate::format::decode_all;
use crate::limits::RequestLimits;

const LAB: u32 = 1001;

/// The client's link, which can go silent: then nothing it sends arrives.
struct Mutable {
    inner: LoopbackTransport,
    muted: Arc<AtomicBool>,
}

impl TransportPort for Mutable {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        if self.muted.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.inner.send_unicast(npdu, mac).await
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

struct Lab {
    hub: Hub<Mutable>,
    engine: Arc<Engine<Mutable>>,
    server: BACnetServer<LoopbackTransport>,
    muted: Arc<AtomicBool>,
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ai(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, instance)
}

fn object(object: ObjectIdentifier) -> Point {
    Point {
        device: LAB,
        object,
        watched: Watched::Object,
    }
}

impl Lab {
    async fn start(options: WatchOptions) -> Self {
        let identity = DeviceIdentity::new(LAB, 42).unwrap();
        let mut objects: Vec<Box<dyn BACnetObject>> = Vec::new();
        for instance in 1..=3 {
            let mut point = AnalogInputObject::new(
                instance,
                format!("AI {instance}"),
                EngineeringUnits::PERCENT,
            )
            .unwrap();
            point.set_present_value(10.0);
            objects.push(Box::new(point));
        }
        objects.push(Box::new(NotificationClass::new(0, "Alarms").unwrap()));
        let database = build_database_with_extra(&identity, objects).unwrap();
        let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
        let server = BACnetServer::generic_builder()
            .device(identity)
            .database(database)
            .transport(server_port)
            .build()
            .await
            .unwrap();
        let muted = Arc::new(AtomicBool::new(false));
        let config = ClientConfig {
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            ..ClientConfig::default()
        };
        let link = Mutable {
            inner: client_port,
            muted: Arc::clone(&muted),
        };
        let client = BACnetClient::start(config, link).await.unwrap();
        client.add_device(LAB, &[2]).await.unwrap();
        let engine = Arc::new(Engine::new(
            Arc::new(client),
            Arc::new(RequestLimits::new(0)),
        ));
        let hub = Hub::new(Arc::clone(&engine), options).unwrap();
        Self {
            hub,
            engine,
            server,
            muted,
        }
    }

    /// The device's subscription table.
    async fn table(&self) -> Vec<BACnetCOVSubscription> {
        let value = self
            .server
            .read_local(
                &oid(ObjectType::DEVICE, LAB),
                PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
                None,
            )
            .await
            .unwrap();
        let PropertyValue::ApplicationData(data) = value else {
            panic!("an encoded list: {value:?}");
        };
        let mut entries = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let (entry, next) = decode_cov_subscription(&data, at).unwrap();
            entries.push(entry);
            at = next;
        }
        entries
    }

    async fn stop(self) {
        assert!(self.hub.shutdown(Duration::from_secs(30)).await);
        drop(self.hub);
        let Ok(engine) = Arc::try_unwrap(self.engine) else {
            panic!("the hub let go of the engine");
        };
        let Ok(mut client) = Arc::try_unwrap(engine.client) else {
            panic!("the engine holds the only client");
        };
        client.stop().await.unwrap();
        let mut server = self.server;
        server.stop().await.unwrap();
    }
}

/// Let the lab's tasks run: paused time moves on only when all are idle.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// The Present_Value among a snapshot's rows.
fn present_value(snapshot: &Snapshot) -> Option<f32> {
    let (_, value) = snapshot
        .rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRESENT_VALUE)?;
    match decode_all(value.as_ref().ok()?)?.as_slice() {
        [PropertyValue::Real(value)] => Some(*value),
        _ => None,
    }
}

/// Acceptance: two listeners on one point make one BACnet subscription,
/// finite, and the last one leaving cancels it, as the device's own table
/// shows.
#[tokio::test(start_paused = true)]
async fn two_leases_share_one_subscription_until_the_last_goes() {
    let lab = Lab::start(WatchOptions::default()).await;
    let first = lab.hub.watch(object(ai(1))).unwrap();
    let second = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    let entry = &table[0];
    assert!(entry.time_remaining > 0 && entry.time_remaining <= 300);
    assert_eq!(entry.monitored_property_reference.object_identifier, ai(1));
    let snapshot = first.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov));
    assert_eq!(snapshot.state, State::Live);
    assert_eq!(present_value(&snapshot), Some(10.0));
    assert_eq!(lab.hub.stats().points, 1);
    drop(first);
    settle().await;
    assert_eq!(lab.table().await.len(), 1, "one listener still holds it");
    drop(second);
    settle().await;
    assert!(
        lab.table().await.is_empty(),
        "the last listener cancelled it"
    );
    assert_eq!(lab.hub.stats().tasks, 0);
    lab.stop().await;
}

/// No lifetime-0 subscription can be built: the type refuses zero, and
/// the class table calls zero a write.
#[test]
fn no_lifetime_is_indefinite() {
    assert!(Lifetime::new(0).is_none());
    assert_eq!(Lifetime::new(1).unwrap().secs(), 1);
    assert_eq!(class::of_subscribe(Lifetime::DEFAULT.secs()), Class::Read);
    assert_eq!(class::of_subscribe(0), Class::Write);
    let fast = WatchOptions {
        poll_every: Duration::from_secs(9),
        ..WatchOptions::default()
    };
    assert!(fast.validate().is_err(), "polls go 10 s apart or slower");
}

/// Acceptance: a device that refuses COV for an object is polled, every
/// 10 s, and the snapshot says so.
#[tokio::test(start_paused = true)]
async fn a_refused_subscription_falls_back_to_polling() {
    let lab = Lab::start(WatchOptions::default()).await;
    let class = lab
        .hub
        .watch(object(oid(ObjectType::NOTIFICATION_CLASS, 0)))
        .unwrap();
    settle().await;
    let snapshot = class.snapshot();
    assert_eq!(snapshot.source, Some(Source::Poll), "{snapshot:?}");
    assert!(
        snapshot.note.as_deref().unwrap().starts_with("COV refused"),
        "{snapshot:?}"
    );
    assert!(lab.table().await.is_empty());
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(class.snapshot().polls, 2, "a poll every 10 s");
    assert_eq!(lab.hub.stats().polled, 1);
    lab.stop().await;
}

/// Acceptance: a burst of notifications makes few signals, the folded ones
/// are counted, and a read afterwards has the latest value.
#[tokio::test(start_paused = true)]
async fn a_notification_storm_is_coalesced() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut lease = lab.hub.watch(object(ai(2))).unwrap();
    settle().await;
    let before = lease.snapshot().seq;
    for step in 0..40u8 {
        lab.server
            .set_present_value_local(&ai(2), PropertyValue::Real(f32::from(step) + 20.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snapshot = lease.snapshot();
    assert_eq!(present_value(&snapshot), Some(59.0), "the latest value");
    assert!(snapshot.notifications >= 40, "{snapshot:?}");
    assert!(snapshot.coalesced >= 30, "{snapshot:?}");
    assert!(snapshot.seq > before);
    // The listener sees one pending signal, not forty.
    lease.changed().await;
    let pending = tokio::time::timeout(Duration::from_millis(1), lease.changed()).await;
    assert!(pending.is_err(), "no second signal is queued");
    lab.stop().await;
}

/// Renewal goes before the lifetime ends, and the device's table shows the
/// lifetime restarted.
#[tokio::test(start_paused = true)]
async fn a_subscription_is_renewed_before_it_lapses() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    tokio::time::sleep(Duration::from_secs(239)).await;
    assert_eq!(lease.snapshot().renewals, 0);
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining <= 62, "{remaining}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(lease.snapshot().renewals, 1);
    let remaining = lab.table().await[0].time_remaining;
    assert!(remaining >= 298, "{remaining}");
    lab.stop().await;
}

/// A device that goes silent fails its renewal: the watch says it is
/// unreachable and polls, backing off, and subscribes again once the
/// device answers.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_reported() {
    let lab = Lab::start(WatchOptions::default()).await;
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    lab.muted.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(245)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.state, State::Unreachable, "{snapshot:?}");
    assert_eq!(snapshot.source, Some(Source::Poll));
    lab.muted.store(false, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(320)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Live);
    lab.stop().await;
}

/// SubscribeCOVProperty for a property watch.
#[tokio::test(start_paused = true)]
async fn a_property_watch_subscribes_to_the_property() {
    let lab = Lab::start(WatchOptions::default()).await;
    let point = Point {
        device: LAB,
        object: ai(3),
        watched: Watched::Property {
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
        },
    };
    let lease = lab.hub.watch(point).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1);
    assert_eq!(
        table[0].monitored_property_reference.property_identifier,
        PropertyIdentifier::PRESENT_VALUE.to_raw()
    );
    lab.server
        .set_present_value_local(&ai(3), PropertyValue::Real(33.0))
        .await
        .unwrap();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(present_value(&snapshot), Some(33.0));
    assert_eq!(snapshot.rows.len(), 1, "only the watched property");
    drop(lease);
    settle().await;
    assert!(lab.table().await.is_empty());
    lab.stop().await;
}

/// The cap counts points still cancelling; a listener joining as the last
/// one leaves gets a fresh entry, and only its subscription remains.
#[tokio::test(start_paused = true)]
async fn the_cap_holds_and_a_rejoin_gets_a_fresh_entry() {
    let options = WatchOptions {
        max_points: 2,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let one = lab.hub.watch(object(ai(1))).unwrap();
    let _two = lab.hub.watch(object(ai(2))).unwrap();
    assert_eq!(
        lab.hub.watch(object(ai(3))).unwrap_err(),
        WatchError::Full { max: 2 }
    );
    settle().await;
    drop(one);
    // Still cancelling: the cap counts it.
    assert!(lab.hub.watch(object(ai(3))).is_err());
    let again = lab.hub.watch(object(ai(1)));
    assert!(again.is_err(), "the old entry is still cancelling");
    settle().await;
    let again = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 2, "{table:?}");
    assert_eq!(again.snapshot().source, Some(Source::Cov));
    lab.stop().await;
}

/// Shutdown cancels every subscription and refuses new watches.
#[tokio::test(start_paused = true)]
async fn shutdown_cancels_every_subscription() {
    let lab = Lab::start(WatchOptions::default()).await;
    let a = lab.hub.watch(object(ai(1))).unwrap();
    let b = lab.hub.watch(object(ai(2))).unwrap();
    settle().await;
    assert_eq!(lab.table().await.len(), 2);
    assert!(lab.hub.shutdown(Duration::from_secs(30)).await);
    assert!(lab.table().await.is_empty());
    assert_eq!(a.snapshot().state, State::Stopped);
    assert_eq!(lab.hub.watch(object(ai(3))).unwrap_err(), WatchError::Shut);
    drop((a, b));
    lab.stop().await;
}

/// A listener joining as the last one leaves, in the same instant: the old
/// entry cancels its own subscription, the new one keeps its own, and the
/// new lease goes on receiving notifications.
#[tokio::test(start_paused = true)]
async fn joining_as_the_last_leaves_keeps_one_live_subscription() {
    let lab = Lab::start(WatchOptions::default()).await;
    let old = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    drop(old);
    let new = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    assert_eq!(new.snapshot().source, Some(Source::Cov));
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(42.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(present_value(&new.snapshot()), Some(42.0));
    lab.stop().await;
}
