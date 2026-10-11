//! The replies the B/IPv6 receive path owes (#1632): the NAKs a node that
//! isn't a BBMD sends (U.4.4, U.4.5.3, U.4.5.4), and the
//! Address-Resolution-ACK for an Address-Resolution or, from a foreign
//! device's BBMD, a Forwarded-Address-Resolution that names this node (U.5).
//! Datagrams are fed in-process with the addressing the OS would report, and
//! each test reads the reply `handle_datagram` returns for the loop to send.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::frame::encoded;
use super::receive::{Receiver, Reply};
use super::reply_limit::{REPLIES_PER_DESTINATION, REPLIES_PER_WINDOW, REPLY_WINDOW};
use super::socket::Bip6Socket;
use super::vmac_table::VmacTable;
use super::*;
use crate::port::TransportPort;
use crate::udp_metadata::ReceivedDatagram;

const LOCAL_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x32]);
const ASKER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x40]);
const OTHER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x41]);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

fn local_ip() -> Ipv6Addr {
    Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10)
}

/// A node on another link, the source of most requests here.
fn sender() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x30),
        0xBAC0,
        0,
        0,
    )
}

/// The BBMD a foreign-device receiver registers with.
fn bbmd() -> SocketAddrV6 {
    SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0xBAC0, 0, 0)
}

/// The asker a Forwarded-Address-Resolution names, on yet another link.
fn asker() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 9, 0, 0, 0, 0x40),
        0xBAC1,
        0,
        0,
    )
}

/// A receiver for `2001:db8::10`, a foreign device of [`bbmd`] when
/// `foreign`. Its socket is never used: the tests read returned replies.
async fn receiver(foreign: bool) -> Receiver {
    let socket = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
        .await
        .unwrap();
    let (tx, _) = mpsc::channel(4);
    Receiver {
        socket: Arc::new(socket),
        tx,
        local_mac: encode_bip6_mac(local_ip(), 0xBAC0),
        vmac: LOCAL_VMAC,
        local_ip: local_ip(),
        unicast_ips: vec![local_ip()],
        wildcard_bind: false,
        foreign_bbmd: foreign.then(|| (*bbmd().ip(), bbmd().port())),
        vmac_table: VmacTable::new(),
        forwarded_group_origin_drops: Arc::default(),
        replies: Default::default(),
    }
}

fn datagram(data: &[u8], peer: SocketAddrV6, destination: Ipv6Addr) -> ReceivedDatagram {
    ReceivedDatagram {
        len: data.len(),
        peer: SocketAddr::V6(peer),
        destination: IpAddr::V6(destination),
        arrival_index: Some(1),
        os_group_delivery: None,
    }
}

/// The reply to `message` from `peer`, sent to `destination`.
async fn reply(
    receiver: &Receiver,
    message: Bvll6<'_>,
    peer: SocketAddrV6,
    destination: Ipv6Addr,
) -> Option<Reply> {
    let frame = encoded(&message).unwrap();
    receiver
        .handle_datagram(&frame, &datagram(&frame, peer, destination))
        .await
}

fn register() -> Bvll6<'static> {
    Bvll6::RegisterForeignDevice {
        source: ASKER_VMAC,
        ttl: 60,
    }
}

fn delete_entry() -> Bvll6<'static> {
    Bvll6::DeleteForeignDeviceTableEntry {
        source: ASKER_VMAC,
        entry: asker().into(),
    }
}

fn distribute() -> Bvll6<'static> {
    Bvll6::DistributeBroadcastToNetwork {
        original_source: ASKER_VMAC,
        npdu: NPDU,
    }
}

fn address_resolution(target: Bip6Vmac) -> Bvll6<'static> {
    Bvll6::AddressResolution {
        source: ASKER_VMAC,
        target,
    }
}

fn forwarded_resolution(target: Bip6Vmac, asker: SocketAddrV6) -> Bvll6<'static> {
    Bvll6::ForwardedAddressResolution {
        original_source: ASKER_VMAC,
        target,
        original_address: asker.into(),
    }
}

fn ack_to(to: SocketAddrV6) -> Reply {
    Reply {
        to: to.into(),
        message: Bvll6::AddressResolutionAck {
            source: LOCAL_VMAC,
            destination: ASKER_VMAC,
        },
    }
}

/// U.4.4, U.4.5.3, U.4.5.4: each request a BBMD serves gets its own NAK,
/// from this node's VMAC, at the datagram's source, in either mode. The
/// octets are written out from U.2.1.1.
#[tokio::test]
async fn each_bbmd_request_gets_its_nak_at_its_source() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        for (request, code, octets) in [
            (
                register(),
                Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                [0x00, 0x90],
            ),
            (
                delete_entry(),
                Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                [0x00, 0xA0],
            ),
            (
                distribute(),
                Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                [0x00, 0xC0],
            ),
        ] {
            let nak = reply(&receiver, request, sender(), local_ip()).await;
            let expected = Reply {
                to: sender().into(),
                message: Bvll6::BvlcResult {
                    source: LOCAL_VMAC,
                    code,
                },
            };
            assert_eq!(nak, Some(expected), "{request:?} foreign={foreign}");
            let wire = encoded(&expected.message).unwrap();
            assert_eq!(
                wire[..],
                [0x82, 0x00, 0x00, 0x09, 0x00, 0x06, 0x32, octets[0], octets[1]]
            );
        }
    }
}

/// A BBMD request sent to a group, or from an address no node sends from,
/// draws no NAK: one forged multicast must not draw one from every member,
/// and no answer may go to a group.
#[tokio::test]
async fn a_bbmd_request_to_a_group_or_from_no_node_is_not_answered() {
    let receiver = receiver(false).await;
    for request in [register(), delete_entry(), distribute()] {
        assert_eq!(
            reply(
                &receiver,
                request,
                sender(),
                BACNET_IPV6_MULTICAST_SITE_LOCAL
            )
            .await,
            None,
            "{request:?} to a group"
        );
        let frame = encoded(&request).unwrap();
        let mut flagged = datagram(&frame, sender(), local_ip());
        flagged.os_group_delivery = Some(true);
        assert_eq!(receiver.handle_datagram(&frame, &flagged).await, None);
        for source in [
            SocketAddrV6::new("ff02::1".parse().unwrap(), 0xBAC0, 0, 0),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0xBAC0, 0, 0),
            SocketAddrV6::new(*sender().ip(), 0, 0, 0),
        ] {
            assert_eq!(
                reply(&receiver, request, source, local_ip()).await,
                None,
                "{request:?} from {source}"
            );
        }
    }
}

/// U.5: an Address-Resolution for this node's VMAC is answered at its
/// source; one for another VMAC is not, though its sender is learned.
#[tokio::test]
async fn address_resolution_is_answered_only_for_this_vmac() {
    let receiver = receiver(false).await;
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    assert_eq!(
        reply(&receiver, address_resolution(OTHER_VMAC), sender(), group).await,
        None
    );
    assert_eq!(
        receiver.vmac_table.lookup(&ASKER_VMAC).await,
        Some(sender())
    );
    assert_eq!(
        reply(&receiver, address_resolution(LOCAL_VMAC), sender(), group).await,
        Some(ack_to(sender()))
    );
    assert_eq!(
        reply(
            &receiver,
            address_resolution(LOCAL_VMAC),
            sender(),
            local_ip()
        )
        .await,
        None,
        "an Address-Resolution goes to a group"
    );
}

/// U.5: a foreign device answers a Forwarded-Address-Resolution from its
/// BBMD that names its VMAC, at the asker's address the message carries.
#[tokio::test]
async fn a_forwarded_address_resolution_from_the_bbmd_is_answered_at_the_asker() {
    let receiver = receiver(true).await;
    assert_eq!(
        reply(
            &receiver,
            forwarded_resolution(LOCAL_VMAC, asker()),
            bbmd(),
            local_ip()
        )
        .await,
        Some(ack_to(asker()))
    );
}

/// Every other Forwarded-Address-Resolution is dropped: from anyone but the
/// configured BBMD, to a node that isn't a foreign device, sent to a group,
/// naming an unusable asker address, or for another VMAC.
#[tokio::test]
async fn forwarded_address_resolutions_that_are_not_answered() {
    let foreign = receiver(true).await;
    let normal = receiver(false).await;
    let ours = forwarded_resolution(LOCAL_VMAC, asker());

    let other_port = SocketAddrV6::new(*bbmd().ip(), bbmd().port() + 1, 0, 0);
    for source in [sender(), asker(), other_port] {
        assert_eq!(
            reply(&foreign, ours, source, local_ip()).await,
            None,
            "from {source}"
        );
    }
    for destination in [local_ip(), BACNET_IPV6_MULTICAST_SITE_LOCAL] {
        assert_eq!(
            reply(&normal, ours, bbmd(), destination).await,
            None,
            "not a foreign device, to {destination}"
        );
    }
    assert_eq!(
        reply(&foreign, ours, bbmd(), BACNET_IPV6_MULTICAST_SITE_LOCAL).await,
        None,
        "sent to a group"
    );
    let port = asker().port();
    for unusable in [
        SocketAddrV6::new("fe80::40".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, port, 0, 0),
        SocketAddrV6::new("ff02::1".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0),
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0),
        SocketAddrV6::new("::ffff:192.0.2.1".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(*asker().ip(), 0, 0, 0),
    ] {
        assert_eq!(
            reply(
                &foreign,
                forwarded_resolution(LOCAL_VMAC, unusable),
                bbmd(),
                local_ip()
            )
            .await,
            None,
            "asker {unusable}"
        );
    }
    assert_eq!(
        reply(
            &foreign,
            forwarded_resolution(OTHER_VMAC, asker()),
            bbmd(),
            local_ip()
        )
        .await,
        None,
        "for another VMAC"
    );
    // The same frame from the BBMD is answered: each refusal above was for
    // the reason it names.
    assert_eq!(
        reply(&foreign, ours, bbmd(), local_ip()).await,
        Some(ack_to(asker()))
    );
}

/// No reply goes to an address that neither sent the datagram nor was named
/// by the configured BBMD in a Forwarded-Address-Resolution, whatever the
/// message, sender, destination or mode. The window is moved on after each
/// datagram, so the quota never hides a reply.
#[tokio::test(start_paused = true)]
async fn no_reply_goes_where_neither_the_sender_nor_the_bbmd_pointed() {
    let stranger = SocketAddrV6::new("2001:db8:0:7::66".parse().unwrap(), 0xBAC0, 0, 0);
    let messages = [
        register(),
        delete_entry(),
        distribute(),
        address_resolution(LOCAL_VMAC),
        address_resolution(OTHER_VMAC),
        forwarded_resolution(LOCAL_VMAC, asker()),
        forwarded_resolution(LOCAL_VMAC, stranger),
        forwarded_resolution(OTHER_VMAC, asker()),
        Bvll6::VirtualAddressResolution { source: ASKER_VMAC },
        Bvll6::AddressResolutionAck {
            source: ASKER_VMAC,
            destination: LOCAL_VMAC,
        },
        Bvll6::ForwardedNpdu {
            original_source: ASKER_VMAC,
            original_address: stranger.into(),
            npdu: NPDU,
        },
        Bvll6::OriginalBroadcastNpdu {
            source: ASKER_VMAC,
            npdu: NPDU,
        },
    ];
    let (mut to_sender, mut to_named) = (0, 0);
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        for message in messages {
            for peer in [sender(), bbmd(), stranger] {
                for destination in [local_ip(), BACNET_IPV6_MULTICAST_SITE_LOCAL] {
                    tokio::time::advance(REPLY_WINDOW).await;
                    let Some(reply) = reply(&receiver, message, peer, destination).await else {
                        continue;
                    };
                    if reply.to == SocketAddr::V6(peer) {
                        to_sender += 1;
                        continue;
                    }
                    let named = matches!(
                        message,
                        Bvll6::ForwardedAddressResolution { original_address, .. }
                            if reply.to == SocketAddr::V6(original_address.into())
                    );
                    assert!(
                        foreign && peer == bbmd() && named,
                        "{message:?} from {peer} to {destination} answered at {}",
                        reply.to
                    );
                    to_named += 1;
                }
            }
        }
    }
    assert!(to_sender > 0 && to_named > 0, "{to_sender} {to_named}");
}

/// The quota is per destination address, shared by the NAKs and the
/// answers to a Forwarded-Address-Resolution sent there, and totalled across
/// destinations; a new window starts it afresh. Address-Resolution and
/// Virtual-Address-Resolution answers stay outside it.
#[tokio::test(start_paused = true)]
async fn replies_stop_at_the_quota() {
    let receiver = receiver(true).await;
    for _ in 0..REPLIES_PER_DESTINATION {
        assert!(reply(&receiver, register(), sender(), local_ip())
            .await
            .is_some());
    }
    assert_eq!(
        reply(&receiver, register(), sender(), local_ip()).await,
        None
    );
    assert_eq!(
        reply(
            &receiver,
            forwarded_resolution(LOCAL_VMAC, sender()),
            bbmd(),
            local_ip()
        )
        .await,
        None,
        "an answer the BBMD asks for shares the quota of the address it goes to"
    );
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    assert_eq!(
        reply(&receiver, address_resolution(LOCAL_VMAC), sender(), group).await,
        Some(ack_to(sender())),
        "a collision probe is still answered"
    );
    let var = Bvll6::VirtualAddressResolution { source: ASKER_VMAC };
    assert!(reply(&receiver, var, sender(), local_ip()).await.is_some());
    assert!(
        reply(&receiver, register(), asker(), local_ip())
            .await
            .is_some(),
        "another address has its own"
    );

    tokio::time::advance(REPLY_WINDOW).await;
    assert!(reply(&receiver, register(), sender(), local_ip())
        .await
        .is_some());

    tokio::time::advance(REPLY_WINDOW).await;
    let mut admitted = 0;
    for host in 0..=u16::try_from(REPLIES_PER_WINDOW / REPLIES_PER_DESTINATION).unwrap() {
        let source = SocketAddrV6::new(
            Ipv6Addr::new(0x2001, 0xdb8, 0, 3, 0, 0, 0, host),
            0xBAC0,
            0,
            0,
        );
        for _ in 0..REPLIES_PER_DESTINATION {
            admitted += u32::from(
                reply(&receiver, delete_entry(), source, local_ip())
                    .await
                    .is_some(),
            );
        }
    }
    assert_eq!(admitted, REPLIES_PER_WINDOW, "the total caps every address");
}

/// End to end on loopback: a running transport sends a Register-Foreign-
/// Device NAK back to the requesting socket, and the loop keeps running.
#[tokio::test]
async fn a_running_transport_naks_a_registration() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x0632));
    let _rx = transport.start().await.unwrap();
    let (_, port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let to = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let requester = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut buf = [0u8; 64];
    for (request, code) in [
        (register(), Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK),
        (
            distribute(),
            Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        ),
    ] {
        let frame: BytesMut = encoded(&request).unwrap();
        requester.send_to(&frame, to).await.unwrap();
        let (len, from) =
            tokio::time::timeout(Duration::from_secs(2), requester.recv_from(&mut buf))
                .await
                .expect("the NAK arrives")
                .unwrap();
        assert_eq!(from, SocketAddr::V6(to));
        assert_eq!(
            decode_bvll6(&buf[..len]).unwrap(),
            Bvll6::BvlcResult {
                source: LOCAL_VMAC,
                code,
            }
        );
    }
    transport.stop().await.unwrap();
}
