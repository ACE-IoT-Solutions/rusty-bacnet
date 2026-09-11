use super::*;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::mstp::LoopbackSerial;
use bacnet_types::enums::{IPMode, NetworkType};

fn ipv4_network_port() -> (ObjectIdentifier, NetworkPortObject) {
    let mut port = NetworkPortObject::new(1, "Primary B/IP", NetworkType::IPV4.to_raw()).unwrap();
    port.set_network_number(42);
    port.set_ip_address(vec![192, 0, 2, 99]);
    port.set_ip_default_gateway(vec![192, 0, 2, 1]);
    port.set_ip_subnet_mask(vec![255, 255, 255, 0]);
    port.set_udp_port(0xBAC0);
    port.set_link_speed(100_000_000.0);
    (port.object_identifier(), port)
}

fn read<T: TransportPort>(
    server: &BACnetServer<T>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let db = server.db.try_read().unwrap();
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

#[tokio::test]
async fn bip_network_port_publishes_ephemeral_identity_and_clears_on_stop() {
    let (oid, port) = ipv4_network_port();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(port)).unwrap();
    let transport: AnyTransport<LoopbackSerial> =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST).into();
    let mut server = BACnetServer::start_clockless(ServerConfig::default(), db, transport)
        .await
        .unwrap();

    let actual_mac = server.local_mac().to_vec();
    let actual_port = u16::from_be_bytes([actual_mac[4], actual_mac[5]]);
    assert_ne!(actual_port, 0);
    assert_eq!(
        read(&server, oid, PropertyIdentifier::MAC_ADDRESS),
        PropertyValue::OctetString(actual_mac)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IP_ADDRESS),
        PropertyValue::OctetString(vec![127, 0, 0, 1])
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::BACNET_IP_UDP_PORT),
        PropertyValue::Unsigned(actual_port as u64)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IP_DEFAULT_GATEWAY),
        PropertyValue::OctetString(vec![192, 0, 2, 1])
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::NETWORK_NUMBER),
        PropertyValue::Unsigned(42)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::BACNET_IP_MODE),
        PropertyValue::Enumerated(IPMode::NORMAL.to_raw())
    );

    server.stop().await.unwrap();
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IP_ADDRESS),
        PropertyValue::OctetString(vec![192, 0, 2, 99])
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::BACNET_IP_UDP_PORT),
        PropertyValue::Unsigned(0xBAC0)
    );
}

#[tokio::test]
async fn bbmd_network_port_refreshes_live_tables_acceptance_and_fdt_countdown() {
    let (oid, port) = ipv4_network_port();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(port)).unwrap();

    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.enable_bbmd(vec![BdtEntry {
        ip: [192, 0, 2, 17],
        port: 47808,
        broadcast_mask: [255, 255, 255, 255],
    }]);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    transport.set_bbmd_accept_foreign_devices(true).unwrap();
    let control = transport.bbmd_control().unwrap();

    let mut server = BACnetServer::start_clockless(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    assert_eq!(
        read(&server, oid, PropertyIdentifier::BACNET_IP_MODE),
        PropertyValue::Enumerated(IPMode::BBMD.to_raw())
    );
    assert_eq!(
        read(
            &server,
            oid,
            PropertyIdentifier::BBMD_ACCEPT_FD_REGISTRATIONS
        ),
        PropertyValue::Boolean(true)
    );
    let PropertyValue::List(initial_bdt) = read(
        &server,
        oid,
        PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE,
    ) else {
        panic!("expected BDT list");
    };
    assert_eq!(initial_bdt.len(), 2, "peer plus auto local BBMD entry");

    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let server_port = u16::from_be_bytes([server.local_mac()[4], server.local_mac()[5]]);
    socket
        .send_to(
            &[0x81, 0x05, 0x00, 0x06, 0x00, 0x02],
            (Ipv4Addr::LOCALHOST, server_port),
        )
        .await
        .unwrap();
    let mut result = [0u8; 6];
    tokio::time::timeout(Duration::from_secs(1), socket.recv(&mut result))
        .await
        .unwrap()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let PropertyValue::List(first_fdt) =
        read(&server, oid, PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE)
    else {
        panic!("expected FDT list");
    };
    let PropertyValue::OctetString(first_row) = &first_fdt[0] else {
        panic!("expected encoded FDT entry");
    };
    let first_remaining = u16::from_be_bytes([first_row[8], first_row[9]]);

    control.set_accept_foreign_devices(false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        read(
            &server,
            oid,
            PropertyIdentifier::BBMD_ACCEPT_FD_REGISTRATIONS
        ),
        PropertyValue::Boolean(false)
    );
    let PropertyValue::List(second_fdt) =
        read(&server, oid, PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE)
    else {
        panic!("expected FDT list");
    };
    let PropertyValue::OctetString(second_row) = &second_fdt[0] else {
        panic!("expected encoded FDT entry");
    };
    let second_remaining = u16::from_be_bytes([second_row[8], second_row[9]]);
    assert!(second_remaining < first_remaining);

    server.stop().await.unwrap();
}
