use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::frame::encoded;
use super::ingress::{original_destination_matches, LocalBinding};
use super::vmac_table::derive_vmac_from_device_instance;
use super::*;
use crate::port::TransportPort;

fn vmac(octets: [u8; 3]) -> Bip6Vmac {
    Bip6Vmac::new(octets)
}

/// `message` on the wire.
fn wire(message: Bvll6<'_>) -> BytesMut {
    encoded(&message).expect("a valid B/IPv6 message")
}

fn binding(
    ip: Ipv6Addr,
    vmac: Bip6Vmac,
    unicast_ips: &[Ipv6Addr],
    wildcard_bind: bool,
) -> LocalBinding<'_> {
    LocalBinding {
        ip,
        vmac,
        unicast_ips,
        wildcard_bind,
    }
}

#[test]
fn original_function_requires_matching_ip_destination_and_vmac() {
    let local_ip = Ipv6Addr::LOCALHOST;
    let local_vmac = vmac([1, 2, 3]);

    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        local_ip.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        local_ip.into(),
        Some(vmac([9, 9, 9])),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_BROADCAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION_ACK,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));

    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9).into(),
        Some(local_vmac),
        &binding(
            local_ip,
            local_vmac,
            &[local_ip, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9),],
            true
        ),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], true),
        Some(true),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION,
        local_ip.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(false),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
        local_ip.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(false),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION,
        local_ip.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(false),
    ));
}

#[tokio::test]
async fn vmac_table_preserves_scope_and_replaces_address_owner() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    let first = SocketAddrV6::new(ip, 47808, 0, 4);
    let replacement = SocketAddrV6::new(ip, 47808, 0, 4);

    table.learn(vmac([1, 2, 3]), first).await;
    assert_eq!(
        table.resolve_by_addr(ip, 47808).await,
        Some((vmac([1, 2, 3]), first))
    );

    table.learn(vmac([4, 5, 6]), replacement).await;
    assert_eq!(table.lookup(&vmac([1, 2, 3])).await, None);
    assert_eq!(
        table.resolve_by_addr(ip, 47808).await,
        Some((vmac([4, 5, 6]), replacement))
    );
}

#[tokio::test]
async fn vmac_table_fails_closed_for_ambiguous_scope() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    table
        .learn(vmac([1, 2, 3]), SocketAddrV6::new(ip, 47808, 0, 4))
        .await;
    table
        .learn(vmac([4, 5, 6]), SocketAddrV6::new(ip, 47808, 0, 7))
        .await;

    assert_eq!(table.resolve_by_addr(ip, 47808).await, None);
}

#[tokio::test]
async fn vmac_table_is_bounded() {
    let table = super::vmac_table::VmacTable::new();
    for index in 0..super::vmac_table::MAX_VMAC_TABLE_ENTRIES {
        let octets = [
            ((index >> 16) & 0xFF) as u8,
            ((index >> 8) & 0xFF) as u8,
            (index & 0xFF) as u8,
        ];
        table
            .learn(
                vmac(octets),
                SocketAddrV6::new(Ipv6Addr::LOCALHOST, 10_000 + index as u16, 0, 0),
            )
            .await;
    }
    table
        .learn(
            vmac([0xFE, 0xFE, 0xFE]),
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 9999, 0, 0),
        )
        .await;

    assert_eq!(table.len().await, super::vmac_table::MAX_VMAC_TABLE_ENTRIES);
    assert_eq!(table.lookup(&vmac([0xFE, 0xFE, 0xFE])).await, None);

    table
        .learn(
            vmac([0xFD, 0xFD, 0xFD]),
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 10_000, 0, 0),
        )
        .await;
    assert_eq!(table.len().await, super::vmac_table::MAX_VMAC_TABLE_ENTRIES);
    assert_eq!(table.lookup(&vmac([0, 0, 0])).await, None);
    assert!(table.lookup(&vmac([0xFD, 0xFD, 0xFD])).await.is_some());
}

#[tokio::test]
async fn vmac_table_does_not_retain_ambiguous_reverse_entries() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    for port in 10_000..10_100 {
        table
            .learn(vmac([1, 2, 3]), SocketAddrV6::new(ip, port, 0, 4))
            .await;
        table
            .learn(vmac([4, 5, 6]), SocketAddrV6::new(ip, port, 0, 7))
            .await;
        assert_eq!(table.resolve_by_addr(ip, port).await, None);
    }

    assert_eq!(table.len().await, 2);
    assert_eq!(table.resolve_by_addr(ip, 10_000).await, None);
}

#[tokio::test]
async fn wrong_destination_vmac_ack_is_not_learned() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let sender_addr = match sender.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let foreign_destination = vmac([9, 9, 9]);

    let ar_source = vmac([1, 2, 3]);
    sender
        .send_to(
            &wire(Bvll6::AddressResolutionAck {
                source: ar_source,
                destination: foreign_destination,
            }),
            destination,
        )
        .await
        .unwrap();
    let var_source = vmac([4, 5, 6]);
    sender
        .send_to(
            &wire(Bvll6::VirtualAddressResolutionAck {
                source: var_source,
                destination: foreign_destination,
            }),
            destination,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(transport.vmac_table.lookup(&ar_source).await, None);
    assert_eq!(transport.vmac_table.lookup(&var_source).await, None);
    assert_eq!(
        transport
            .vmac_table
            .resolve_by_addr(*sender_addr.ip(), sender_addr.port())
            .await,
        None
    );

    transport.stop().await.unwrap();
}

#[test]
fn bip6_max_apdu_length() {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    assert_eq!(transport.egress_apdu_limit(), 1476);
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
}

#[tokio::test]
async fn bip6_start_stop() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1));
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    assert_eq!(transport.local_mac().len(), 18);
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn bip6_unicast_loopback() {
    let mut transport_a = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(2));
    let mut transport_b = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(3));

    let _rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD];

    let error = transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("destination VMAC is unknown"));

    let (peer_ip, peer_port) = decode_bip6_mac(transport_b.local_mac()).unwrap();
    transport_a
        .vmac_table
        .learn(
            transport_b.source_vmac,
            SocketAddrV6::new(peer_ip, peer_port, 0, 0),
        )
        .await;

    transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap();

    let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx_b.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

    transport_a.stop().await.unwrap();
    transport_b.stop().await.unwrap();
}

#[test]
fn vmac_from_valid_device_instance_uses_all_22_bits() {
    let vmac = derive_vmac_from_device_instance(0x123456);
    assert_eq!(vmac.octets(), [0x12, 0x34, 0x56]);
    assert_eq!(
        derive_vmac_from_device_instance(0x3F_FFFF).octets(),
        [0x3F, 0xFF, 0xFF]
    );
}

#[tokio::test]
async fn bip6_forwarded_npdu_delivered() {
    // Verify a foreign device accepts a ForwardedNpdu only from its BBMD.
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let sender_addr = match sender.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(44));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *sender_addr.ip(),
        bbmd_port: sender_addr.port(),
        ttl: 60,
    });
    let mut rx = transport.start().await.unwrap();

    // Build a ForwardedNpdu frame from a "BBMD"
    let originating_vmac = vmac([0xAA, 0xAA, 0xAA]);
    let test_npdu = vec![0x01, 0x00, 0xCA, 0xFE];

    let origin_addr =
        SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9), 47_808, 0, 0);
    let forwarded = |source: Bip6Vmac, origin: SocketAddrV6, npdu: &[u8]| {
        wire(Bvll6::ForwardedNpdu {
            original_source: source,
            original_address: origin.into(),
            npdu,
        })
    };
    let buf = forwarded(originating_vmac, origin_addr, &test_npdu);

    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);

    let malformed_vmac = vmac([0x41, 0x00, 0x01]);
    let malformed = forwarded(malformed_vmac, origin_addr, &[0]);
    sender.send_to(&malformed, dest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(transport.vmac_table.lookup(&malformed_vmac).await, None);

    let invalid_origins = [
        (
            vmac([0x41, 0x00, 0x02]),
            SocketAddrV6::new(
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                origin_addr.port(),
                0,
                4,
            ),
        ),
        (
            vmac([0x41, 0x00, 0x03]),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x04]),
            SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x05]),
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x06]),
            SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 10), 0, 0, 0),
        ),
    ];
    for (vmac, invalid_origin) in invalid_origins {
        let frame = forwarded(vmac, invalid_origin, &test_npdu);
        sender.send_to(&frame, dest).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    for (vmac, _) in invalid_origins {
        assert_eq!(transport.vmac_table.lookup(&vmac).await, None);
    }

    sender.send_to(&buf, dest).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip6_mac(*origin_addr.ip(), origin_addr.port())
    );
    assert!(received.link_layer_group);

    assert_eq!(
        transport.vmac_table.lookup(&originating_vmac).await,
        Some(origin_addr)
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bip6_var_response() {
    // A unicast VAR asks the destination node to disclose its VMAC.
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let our_vmac = transport.source_vmac;
    let requester_vmac = vmac([0x41, 0x22, 0x33]);

    let buf = wire(Bvll6::VirtualAddressResolution {
        source: requester_vmac,
    });

    // Send VAR to the transport
    let checker = UdpSocket::bind("[::1]:0").await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    checker.send_to(&buf, dest).await.unwrap();

    // We should receive a VAR-Ack back (confirming collision)
    let mut resp_buf = vec![0u8; 64];
    let result =
        tokio::time::timeout(Duration::from_secs(2), checker.recv_from(&mut resp_buf)).await;

    match result {
        Ok(Ok((len, _))) => {
            let frame = decode_bvll6(&resp_buf[..len]).unwrap();
            assert_eq!(
                frame.function(),
                Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
            );
            assert_eq!(frame.source_vmac(), our_vmac);
            assert_eq!(frame.destination_vmac(), Some(requester_vmac));
        }
        Ok(Err(e)) => panic!("recv error: {e}"),
        Err(_) => panic!("timeout waiting for VAR-Ack response"),
    }

    transport.stop().await.unwrap();
}
