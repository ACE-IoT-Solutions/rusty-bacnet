//! A `BACnetClient` on a `BACnetRouter`'s own application entity (#1808),
//! through `RouterApplicationPort`, on in-process loopback links:
//!
//! - network 1 holds a silent station at MAC 7, which the tests name as the
//!   next router to network 3, a network nobody answers on;
//! - network 2 holds a `BACnetServer` as station 5.
//!
//! The client finds the server with a global Who-Is, reads it through the
//! routing table, takes a segmented reply, runs requests side by side, times
//! out toward network 3, and hears a network the table lacks refused the way
//! `BACnetRouter::originate` refuses it.

use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{
    BACnetRouter, OriginateError, OriginateFailure, RouterApplicationPort, RouterOptions,
    RouterPort,
};
use bacnet_network::router_table::PortId;
use bacnet_npdu::Mac;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    AbortReason, EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::sync::mpsc;

const INSTANCE: u32 = 4321;
const NAME: &str = "Supply Air Temp";
const STATION: u8 = 5;
const SILENT: u8 = 7;
/// The client's advertised APDU limit, small enough that a whole Device
/// object needs several segments.
const CLIENT_MAX_APDU: u16 = 128;
const APDU_TIMEOUT_MS: u64 = 1_000;
const RETRIES: u8 = 2;

struct Site {
    router: BACnetRouter,
    server: BACnetServer<LoopbackTransport>,
    client: BACnetClient<RouterApplicationPort>,
    /// What reaches the silent station on network 1.
    silent: mpsc::Receiver<ReceivedNpdu>,
}

async fn site() -> Site {
    let (near, mut silent) = LoopbackTransport::pair(vec![1], vec![SILENT]);
    let (far, station) = LoopbackTransport::pair(vec![2], vec![STATION]);
    let ports = vec![
        RouterPort {
            transport: near,
            network_number: 1,
        },
        RouterPort {
            transport: far,
            network_number: 2,
        },
    ];
    let started = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let silent = silent.start().await.unwrap();
    let server = server(station).await;
    let port = started.router.application_port(started.apdus);
    let client = BACnetClient::generic_builder()
        .transport(port)
        .apdu_timeout_ms(APDU_TIMEOUT_MS)
        .apdu_retries(RETRIES)
        .max_apdu_length(CLIENT_MAX_APDU)
        .build()
        .await
        .unwrap();
    let mut site = Site {
        router: started.router,
        server,
        client,
        silent,
    };
    // Let the router's start-up announcements go by.
    tokio::time::sleep(Duration::from_millis(100)).await;
    while site.silent.try_recv().is_ok() {}
    site
}

impl Site {
    async fn stop(mut self) {
        self.client.stop().await.unwrap();
        self.server.stop().await.unwrap();
        self.router.stop().await.unwrap();
    }
}

async fn server(transport: LoopbackTransport) -> BACnetServer<LoopbackTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(INSTANCE, 0)
        .unwrap()
        .with_segmentation(Segmentation::BOTH);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("Air handler"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(3, NAME, EngineeringUnits::DEGREES_FAHRENHEIT).unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .build()
        .await
        .unwrap()
}

fn supply_air() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap()
}

fn character_string(encoded: &[u8]) -> PropertyValue {
    bacnet_encoding::primitives::decode_application_value(encoded, 0)
        .unwrap()
        .0
}

/// The read the routed tests make: Supply Air Temp's name, from station
/// `station` on `network`. The router MAC is ignored by the port.
async fn read_name(
    client: &BACnetClient<RouterApplicationPort>,
    network: u16,
    station: u8,
) -> Result<PropertyValue, Error> {
    let ack = client
        .read_property_routed(
            &[0xEE],
            network,
            &[station],
            supply_air(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await?;
    Ok(character_string(&ack.property_value))
}

#[tokio::test(start_paused = true)]
async fn the_routers_client_reads_a_device_through_its_table() {
    let site = site().await;
    let name = read_name(&site.client, 2, STATION).await.unwrap();
    assert_eq!(name, PropertyValue::CharacterString(NAME.into()));

    // Side by side: two transactions in flight at once, each matched to
    // its own reply.
    let (first, second) = tokio::join!(
        read_name(&site.client, 2, STATION),
        site.client.read_property_routed(
            &[0xEE],
            2,
            &[STATION],
            device(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
    );
    assert_eq!(first.unwrap(), PropertyValue::CharacterString(NAME.into()));
    assert_eq!(
        character_string(&second.unwrap().property_value),
        PropertyValue::CharacterString("Air handler".into())
    );
    assert_eq!(site.router.counters().originated, 3);
    site.stop().await;
}

/// A global Who-Is leaves by every port; the I-Am comes back from network
/// 2, which the client keeps as the device's network.
#[tokio::test(start_paused = true)]
async fn a_device_found_by_who_is_is_read_by_its_instance() {
    let mut site = site().await;
    site.client.who_is(None).await.unwrap();
    let found = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(found) = site.client.get_device(INSTANCE).await {
                break found;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the I-Am arrives");
    assert_eq!(found.source_network, Some(2));
    assert_eq!(found.source_address.as_deref(), Some(&[STATION][..]));
    // Network 1's silent station heard the Who-Is too.
    let who_is = next_frame(&mut site.silent).await;
    assert_eq!(&who_is[..6], &[1, 0x20, 0xFF, 0xFF, 0, 255]);

    let ack = site
        .client
        .read_property_from_device(
            INSTANCE,
            supply_air(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        character_string(&ack.property_value),
        PropertyValue::CharacterString(NAME.into())
    );
    site.stop().await;
}

/// The whole Device object doesn't fit the client's 128-octet APDUs: the
/// server segments its reply and the client's TSM acknowledges each
/// window back through the router.
#[tokio::test(start_paused = true)]
async fn a_segmented_reply_is_reassembled() {
    let site = site().await;
    site.client.who_is(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while site.client.get_device(INSTANCE).await.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the I-Am arrives");
    let originated = site.router.counters().originated;
    let ack = site
        .client
        .read_property_multiple_from_device(
            INSTANCE,
            vec![ReadAccessSpecification {
                object_identifier: device(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::ALL,
                    property_array_index: None,
                }],
            }],
        )
        .await
        .unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    let encoded: usize = results
        .iter()
        .map(|result| {
            result
                .property_value
                .as_ref()
                .map_or(0, |value| value.len())
        })
        .sum();
    assert!(
        encoded > usize::from(CLIENT_MAX_APDU),
        "{encoded} octets of values"
    );
    // The request, then at least one Segment-ACK per further segment.
    let sent = site.router.counters().originated - originated;
    assert!(sent >= 3, "{sent} originated");
    site.stop().await;
}

/// A route the table holds, to a network where nothing answers: each
/// attempt goes to the next router, and the request times out.
#[tokio::test(start_paused = true)]
async fn a_request_toward_a_silent_network_times_out() {
    let mut site = site().await;
    let now = site.router.now();
    assert!(
        site.router
            .table()
            .add_learned(3, PortId(0), Mac::from_array([SILENT]), now)
    );
    let error = read_name(&site.client, 3, 9).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Remote(RemoteError::Abort {
                reason: AbortReason::TSM_TIMEOUT
            })
        ),
        "{error:?}"
    );
    // The first attempt and each retry, to network 3 by station 7.
    let mut attempts = 0;
    while let Ok(frame) = site.silent.try_recv() {
        assert_eq!(&frame.npdu[..6], &[1, 0x24, 0, 3, 1, 9]);
        attempts += 1;
    }
    assert_eq!(attempts, 1 + usize::from(RETRIES));
    site.stop().await;
}

/// A network the table lacks: refused at once as `originate` refuses it,
/// with the search for it started.
#[tokio::test(start_paused = true)]
async fn an_unknown_network_is_refused_as_the_router_refuses_it() {
    let mut site = site().await;
    let error = read_name(&site.client, 4, 9).await.unwrap_err();
    assert_eq!(
        OriginateFailure::from_error(&error),
        Some(OriginateFailure::Refused(OriginateError::NoRoute)),
        "{error:?}"
    );
    // Who-Is-Router-To-Network 4, on network 1 as on every port.
    assert_eq!(
        next_frame(&mut site.silent).await,
        [1, 0x80, 0x00, 0x00, 0x04]
    );
    site.stop().await;
}

/// The next NPDU to reach the silent station.
async fn next_frame(silent: &mut mpsc::Receiver<ReceivedNpdu>) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), silent.recv())
        .await
        .expect("a frame arrives")
        .expect("the link is open")
        .npdu
        .to_vec()
}
