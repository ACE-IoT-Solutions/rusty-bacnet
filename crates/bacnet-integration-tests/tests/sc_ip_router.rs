//! End-to-end mixed-transport routing evidence.
//!
//! A real BACnet/IP client sends a routed ReadProperty request through a
//! `BACnetRouter` whose remote port is connected to a mutual-TLS BACnet/SC
//! hub. The target is a real BACnet/SC server behind that hub.

use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_network::router::{BACnetRouter, RouterPort};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::NoSerial;
use bacnet_transport::port::TransportHealthState;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use tokio::time::{timeout, Duration};
use tokio_rustls::rustls::{self, pki_types::PrivatePkcs8KeyDer};

const BIP_NETWORK: u16 = 1001;
const SC_NETWORK: u16 = 2001;
const HUB_VMAC: [u8; 6] = [0x02, 0, 0, 0, 0, 1];
const ROUTER_SC_VMAC: [u8; 6] = [0x02, 0, 0, 0, 0, 2];
const SERVER_SC_VMAC: [u8; 6] = [0x02, 0, 0, 0, 0, 3];
const HUB_UUID: [u8; 16] = [0x11; 16];
const ROUTER_SC_UUID: [u8; 16] = [0x22; 16];
const SERVER_SC_UUID: [u8; 16] = [0x33; 16];

struct TlsFixture {
    hub: ScHubTlsConfig,
    router: ScNodeTlsConfig,
    server: ScNodeTlsConfig,
}

impl TlsFixture {
    fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let leaf = |name: &str, usage| {
            let mut params = CertificateParams::new(vec![name.to_owned()]).unwrap();
            params.extended_key_usages = vec![usage];
            let key = KeyPair::generate().unwrap();
            let cert = params.signed_by(&key, &issuer).unwrap();
            (
                cert.der().clone(),
                PrivatePkcs8KeyDer::from(key.serialize_der()),
            )
        };

        let (hub_cert, hub_key) = leaf("127.0.0.1", ExtendedKeyUsagePurpose::ServerAuth);
        let (router_cert, router_key) = leaf("router", ExtendedKeyUsagePurpose::ClientAuth);
        let (server_cert, server_key) = leaf("server", ExtendedKeyUsagePurpose::ClientAuth);
        assert_ne!(
            router_cert, server_cert,
            "SC nodes need distinct credentials"
        );

        let trust = vec![ca.der().clone()];
        Self {
            hub: ScHubTlsConfig::from_der(trust.clone(), vec![hub_cert], hub_key.into()).unwrap(),
            router: ScNodeTlsConfig::from_der(trust.clone(), vec![router_cert], router_key.into())
                .unwrap(),
            server: ScNodeTlsConfig::from_der(trust, vec![server_cert], server_key.into()).unwrap(),
        }
    }
}

fn reserve_udp_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn bip_mac(port: u16) -> [u8; 6] {
    let [high, low] = port.to_be_bytes();
    [127, 0, 0, 1, high, low]
}

fn server_database() -> ObjectDatabase {
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 4321,
        name: "SC Router Integration Device".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut input = AnalogInputObject::new(7, "SC Zone Temperature", 62).unwrap();
    input.set_present_value(68.25);
    device.set_object_list(vec![device.object_identifier(), input.object_identifier()]);

    let mut database = ObjectDatabase::new();
    database.add(Box::new(device)).unwrap();
    database.add(Box::new(input)).unwrap();
    database
}

#[tokio::test]
async fn bip_client_routes_read_property_to_sc_server_through_mtls_hub() {
    let tls = TlsFixture::new();
    let mut hub = timeout(
        Duration::from_secs(5),
        ScHub::start_with_tls_config(
            "127.0.0.1:0",
            tls.hub,
            HUB_VMAC,
            HUB_UUID,
            ScHubHandshakeTimeouts::default(),
        ),
    )
    .await
    .expect("SC hub startup timed out")
    .expect("SC hub startup failed");
    let hub_url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());

    let mut server = timeout(
        Duration::from_secs(5),
        BACnetServer::sc_builder()
            .hub_url(&hub_url)
            .tls_config(tls.server)
            .vmac(SERVER_SC_VMAC)
            .device_uuid(SERVER_SC_UUID)
            .database(server_database())
            .build(),
    )
    .await
    .expect("SC server startup timed out")
    .expect("SC server startup failed");

    let router_ws = timeout(
        Duration::from_secs(5),
        TlsWebSocket::connect(&hub_url, tls.router),
    )
    .await
    .expect("router TLS connection timed out")
    .expect("router TLS connection failed");
    let router_sc = ScTransport::new(router_ws, ROUTER_SC_VMAC).with_device_uuid(ROUTER_SC_UUID);

    let router_udp_port = reserve_udp_port();
    let router_bip = BipTransport::new(Ipv4Addr::LOCALHOST, router_udp_port, Ipv4Addr::LOCALHOST);
    let ports: Vec<RouterPort<AnyTransport<NoSerial>>> = vec![
        RouterPort {
            transport: router_bip.into(),
            network_number: BIP_NETWORK,
        },
        RouterPort {
            transport: router_sc.into(),
            network_number: SC_NETWORK,
        },
    ];
    let (mut router, _local_rx) = timeout(Duration::from_secs(5), BACnetRouter::start(ports))
        .await
        .expect("mixed router startup timed out")
        .expect("mixed router startup failed");

    {
        let table = router.table().lock().await;
        let bip = table.lookup(BIP_NETWORK).expect("B/IP direct route");
        assert!(bip.directly_connected);
        assert_eq!(bip.port_index, 0);
        let sc = table.lookup(SC_NETWORK).expect("SC direct route");
        assert!(sc.directly_connected);
        assert_eq!(sc.port_index, 1);
    }
    let routes = router.routing_table().await;
    assert_eq!(
        routes
            .iter()
            .map(|route| route.network_number)
            .collect::<Vec<_>>(),
        vec![BIP_NETWORK, SC_NETWORK]
    );
    assert!(routes.iter().all(|route| route.directly_connected));

    let health = router.port_health();
    assert_eq!(health.len(), 2);
    assert_eq!(health[0].transport_kind, "bip");
    assert_eq!(health[0].health.state, TransportHealthState::Up);
    assert_eq!(health[1].transport_kind, "sc");
    assert_eq!(health[1].health.state, TransportHealthState::Up);

    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(3_000)
        .build()
        .await
        .unwrap();

    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let ack = timeout(
        Duration::from_secs(5),
        client.read_property_routed(
            &bip_mac(router_udp_port),
            SC_NETWORK,
            &SERVER_SC_VMAC,
            object,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("routed ReadProperty timed out")
    .expect("routed ReadProperty failed");
    assert_eq!(ack.object_identifier, object);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    let (value, _) = bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0)
        .expect("decode routed Present_Value");
    assert_eq!(value, PropertyValue::Real(68.25));

    let counters = router.port_counters();
    assert_eq!(counters.len(), 2);
    assert_eq!(counters[0].network_number, BIP_NETWORK);
    assert_eq!(counters[1].network_number, SC_NETWORK);
    assert_eq!(counters[0].forwarded_unicast, 1, "SC response crossed B/IP");
    assert_eq!(counters[1].forwarded_unicast, 1, "B/IP request crossed SC");

    client.stop().await.unwrap();
    router.stop().await;
    server.stop().await.unwrap();
    hub.stop().await;
}
