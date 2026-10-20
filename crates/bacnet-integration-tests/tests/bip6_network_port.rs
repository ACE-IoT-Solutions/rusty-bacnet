//! A registered B/IPv6 Network Port on a real `Bip6Transport` (#1579), read
//! by a B/IPv6 client over loopback: startup publishes the bound port, the
//! VMAC and the mode, NORMAL or FOREIGN, and the port serves the IPV6 rows
//! (Table 12-72) with that mode's own.
#![cfg(feature = "ipv6")]

use std::net::{Ipv6Addr, SocketAddrV6};

use bacnet_client::client::BACnetClient;
use bacnet_encoding::constructed::decode_host_n_port;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::network_port::{Bip6PortConfig, NetworkPortObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip6::{Bip6ForeignDeviceConfig, Bip6Transport};
use bacnet_types::constructed::BACnetHostNPort;
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

const SERVER_INSTANCE: u32 = 1234;
/// The VMAC of Device 1234 (Clause H.7.2).
const SERVER_VMAC: [u8; 3] = [0x00, 0x04, 0xD2];

fn port() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

async fn server(transport: Bip6Transport) -> BACnetServer<Bip6Transport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip6(
            1,
            "B/IPv6",
            Bip6PortConfig {
                network_number: 6,
                ip_address: Ipv6Addr::LOCALHOST.octets(),
                udp_port: 0,
                prefix_length: 128,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(SERVER_INSTANCE, 0).unwrap())
        .database(db)
        .transport(transport)
        .registered_network_port(port())
        .build()
        .await
        .unwrap()
}

/// A client that knows the server's VMAC.
async fn client(server: &BACnetServer<Bip6Transport>) -> BACnetClient<Bip6Transport> {
    let client = BACnetClient::generic_builder()
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10)))
        .build()
        .await
        .unwrap();
    let address = server.transport().unwrap().local_address().unwrap();
    let address = SocketAddrV6::new(*address.ip(), address.port(), 0, 0);
    let vmac = timeout(
        Duration::from_secs(5),
        client.transport().resolve_virtual_address(address),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(vmac.octets(), SERVER_VMAC);
    client
}

/// Read one property of the port: its value bytes.
async fn read(client: &BACnetClient<Bip6Transport>, property: P) -> Vec<u8> {
    timeout(
        Duration::from_secs(5),
        client.read_property(&SERVER_VMAC, port(), property, None),
    )
    .await
    .unwrap()
    .unwrap_or_else(|error| panic!("{property}: {error:?}"))
    .property_value
}

/// The value of an application-tagged property.
fn value(bytes: &[u8]) -> PropertyValue {
    decode_application_value(bytes, 0).unwrap().0
}

/// The port's Property_List.
async fn property_list(client: &BACnetClient<Bip6Transport>) -> Vec<P> {
    let bytes = read(client, P::PROPERTY_LIST).await;
    let mut listed = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, end) = decode_application_value(&bytes, offset).unwrap();
        let PropertyValue::Enumerated(raw) = value else {
            panic!("Property_List holds enumerations: {value:?}");
        };
        listed.push(P::from_raw(raw));
        offset = end;
    }
    listed
}

/// The rows every B/IPv6 port serves, whatever its mode, read back as the
/// bound transport is.
async fn assert_ipv6_rows(client: &BACnetClient<Bip6Transport>, bound: u16, mode: u32) {
    assert_eq!(
        value(&read(client, P::NETWORK_TYPE).await),
        PropertyValue::Enumerated(9)
    );
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_MODE).await),
        PropertyValue::Enumerated(mode)
    );
    assert_eq!(
        value(&read(client, P::IPV6_ADDRESS).await),
        PropertyValue::OctetString(Ipv6Addr::LOCALHOST.octets().to_vec())
    );
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_UDP_PORT).await),
        PropertyValue::Unsigned(bound.into())
    );
    assert_eq!(
        value(&read(client, P::MAC_ADDRESS).await),
        PropertyValue::OctetString(SERVER_VMAC.to_vec())
    );
    // The transport's default site-local group, at its bound port.
    let mut group = Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0)
        .octets()
        .to_vec();
    group.extend_from_slice(&bound.to_be_bytes());
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_MULTICAST_ADDRESS).await),
        PropertyValue::OctetString(group)
    );
    assert_eq!(
        value(&read(client, P::NETWORK_NUMBER).await),
        PropertyValue::Unsigned(6)
    );
    let listed = property_list(client).await;
    for property in [
        P::BACNET_IPV6_MODE,
        P::IPV6_ADDRESS,
        P::IPV6_PREFIX_LENGTH,
        P::BACNET_IPV6_UDP_PORT,
        P::IPV6_DEFAULT_GATEWAY,
        P::BACNET_IPV6_MULTICAST_ADDRESS,
        P::IPV6_DNS_SERVER,
    ] {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in [
        P::BACNET_IP_MODE,
        P::IP_ADDRESS,
        P::BBMD_FOREIGN_DEVICE_TABLE,
    ] {
        assert!(!listed.contains(&property), "{property} not listed");
    }
}

#[tokio::test]
async fn a_normal_b_ipv6_server_registers_its_network_port() {
    let mut server = server(Bip6Transport::new(
        Ipv6Addr::LOCALHOST,
        0,
        Some(SERVER_INSTANCE),
    ))
    .await;
    let bound = server.transport().unwrap().local_address().unwrap().port();
    assert_ne!(bound, 0);
    let mut client = client(&server).await;
    assert_ipv6_rows(&client, bound, 0).await;
    let listed = property_list(&client).await;
    assert!(!listed.contains(&P::FD_BBMD_ADDRESS));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_foreign_b_ipv6_server_serves_its_bbmd_and_lifetime() {
    // A BBMD that only listens: the server's registration goes to it.
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let bbmd_port = bbmd.local_addr().unwrap().port();
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(SERVER_INSTANCE));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port,
        ttl: 300,
    });
    let mut server = server(transport).await;
    let bound = server.transport().unwrap().local_address().unwrap().port();
    let mut client = client(&server).await;
    assert_ipv6_rows(&client, bound, 1).await;
    let address = read(&client, P::FD_BBMD_ADDRESS).await;
    let expected = BACnetHostNPort::from_socket_addr(
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, bbmd_port, 0, 0).into(),
    );
    assert_eq!(
        decode_host_n_port(&address, 0).unwrap(),
        (expected, address.len())
    );
    assert_eq!(
        value(&read(&client, P::FD_SUBSCRIPTION_LIFETIME).await),
        PropertyValue::Unsigned(300)
    );
    let listed = property_list(&client).await;
    assert!(listed.contains(&P::FD_SUBSCRIPTION_LIFETIME));
    // The transport can't move its registration, so the rows refuse writes.
    let refused = timeout(
        Duration::from_secs(5),
        client.write_property(
            &SERVER_VMAC,
            port(),
            P::FD_SUBSCRIPTION_LIFETIME,
            None,
            vec![0x21, 60],
            None,
        ),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            refused,
            Err(Error::Protocol {
                code: ErrorCode::WRITE_ACCESS_DENIED,
                ..
            })
        ),
        "{refused:?}"
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
