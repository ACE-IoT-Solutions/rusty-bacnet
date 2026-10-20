//! A running B/IPv6 foreign device moved by its Network Port (#1579): an
//! activated BBMD or time-to-live is registered at once, and broadcasts and
//! the trust of Forwarded-NPDUs follow it, as on B/IP (#1825).

use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bip_port::{BipPortMode, ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::ErrorCode;
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::*;
use crate::port::TransportPort;

async fn bbmd() -> UdpSocket {
    UdpSocket::bind("[::1]:0").await.unwrap()
}

fn address_of(peer: &UdpSocket) -> BACnetHostNPort {
    BACnetHostNPort::from_socket_addr(peer.local_addr().unwrap())
}

/// The next datagram `peer` receives that `accept` takes.
async fn next<T>(peer: &UdpSocket, accept: impl Fn(Bvll6<'_>) -> Option<T>) -> T {
    let mut buf = [0u8; 1600];
    loop {
        let (len, _) = timeout(Duration::from_secs(5), peer.recv_from(&mut buf))
            .await
            .expect("a datagram")
            .unwrap();
        if let Some(found) = decode_bvll6(&buf[..len]).ok().and_then(&accept) {
            return found;
        }
    }
}

/// The time-to-live of the next Register-Foreign-Device `peer` receives.
async fn registration(peer: &UdpSocket) -> u16 {
    next(peer, |message| match message {
        Bvll6::RegisterForeignDevice { ttl, .. } => Some(ttl),
        _ => None,
    })
    .await
}

/// A started foreign device of `peer`, and the registration it lends.
async fn started(peer: &UdpSocket, ttl: u16) -> (Bip6Transport, Arc<dyn ForeignRegistration>) {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(55));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: peer.local_addr().unwrap().port(),
        ttl,
    });
    let BipPortMode::Foreign { registration: lent } = transport.bip6_port().unwrap().mode else {
        panic!("FOREIGN mode");
    };
    let _rx = transport.start().await.unwrap();
    // The registration made at start.
    assert_eq!(registration(peer).await, ttl);
    (transport, lent)
}

fn changes(bbmd: Option<BACnetHostNPort>, ttl: Option<u16>) -> ForeignDeviceChanges {
    let mut changes = ForeignDeviceChanges::default();
    changes.bbmd = bbmd;
    changes.subscription_lifetime = ttl;
    changes
}

fn code(error: Error) -> ErrorCode {
    match error {
        Error::Protocol { code, .. } => code,
        other => panic!("a protocol error: {other:?}"),
    }
}

#[tokio::test]
async fn activation_sends_the_next_registration_to_the_new_bbmd_with_the_new_ttl() {
    let old = bbmd().await;
    let new = bbmd().await;
    let (mut transport, lent) = started(&old, 300).await;
    let moved = changes(Some(address_of(&new)), Some(120));
    lent.check(&moved).unwrap();
    lent.activate(&moved).unwrap();
    // At once, not at the next renewal.
    assert_eq!(registration(&new).await, 120);
    assert_eq!(lent.bbmd(), address_of(&new));
    assert_eq!(lent.subscription_lifetime(), 120);
    let config = transport.foreign_device_config().unwrap();
    assert_eq!(
        (config.bbmd_port, config.ttl),
        (new.local_addr().unwrap().port(), 120)
    );
    // Broadcasts now go to the new BBMD.
    transport.send_broadcast(&[0x01, 0x00, 0x7A]).await.unwrap();
    let npdu = next(&new, |message| match message {
        Bvll6::DistributeBroadcastToNetwork { npdu, .. } => Some(npdu.to_vec()),
        _ => None,
    })
    .await;
    assert_eq!(npdu, [0x01, 0x00, 0x7A]);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn a_refused_value_leaves_the_registration_untouched() {
    let peer = bbmd().await;
    let (mut transport, lent) = started(&peer, 300).await;
    let own = transport.local_address().unwrap();
    let port = peer.local_addr().unwrap().port();
    let at = |ip: Ipv6Addr, port: u16| BACnetHostNPort {
        host: BACnetHostAddress::Ip(ip.into()),
        port,
    };
    let parse = |text: &str| text.parse::<Ipv6Addr>().unwrap();
    for refused in [
        changes(Some(at(Ipv6Addr::UNSPECIFIED, port)), None),
        changes(Some(at(parse("ff05::bac0"), port)), None),
        changes(Some(at(parse("fe80::1"), port)), None),
        changes(Some(at(parse("::ffff:10.0.0.1"), port)), None),
        changes(Some(at(Ipv6Addr::LOCALHOST, 0)), None),
        changes(Some(at(*own.ip(), own.port())), None),
        changes(
            Some(BACnetHostNPort {
                host: BACnetHostAddress::Ip([10, 0, 0, 1].into()),
                port,
            }),
            None,
        ),
        changes(
            Some(BACnetHostNPort {
                host: BACnetHostAddress::Name("bbmd.example".into()),
                port,
            }),
            None,
        ),
        changes(None, Some(0)),
    ] {
        assert_eq!(
            code(lent.check(&refused).unwrap_err()),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "{refused:?}"
        );
        assert_eq!(
            code(lent.activate(&refused).unwrap_err()),
            ErrorCode::VALUE_OUT_OF_RANGE
        );
        assert_eq!(lent.bbmd(), address_of(&peer));
        assert_eq!(lent.subscription_lifetime(), 300);
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn a_change_needs_a_running_transport_and_a_restart_keeps_it() {
    let peer = bbmd().await;
    let (mut transport, lent) = started(&peer, 300).await;
    transport.stop().await.unwrap();
    let error = lent.check(&changes(None, Some(60))).unwrap_err();
    assert_eq!(code(error), ErrorCode::WRITE_ACCESS_DENIED);
    // A later start takes changes again, and keeps what was activated.
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&peer).await, 300);
    lent.activate(&changes(None, Some(60))).unwrap();
    assert_eq!(registration(&peer).await, 60);
    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&peer).await, 60);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn forwarded_npdus_are_trusted_from_the_new_bbmd_only() {
    let old = bbmd().await;
    let new = bbmd().await;
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(56));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: old.local_addr().unwrap().port(),
        ttl: 300,
    });
    let BipPortMode::Foreign { registration: lent } = transport.bip6_port().unwrap().mode else {
        panic!("FOREIGN mode");
    };
    let mut rx = transport.start().await.unwrap();
    let to = SocketAddrV6::new(
        Ipv6Addr::LOCALHOST,
        transport.local_address().unwrap().port(),
        0,
        0,
    );
    lent.activate(&changes(Some(address_of(&new)), None))
        .unwrap();
    let forwarded = |marker: u8| {
        let npdu = [0x01, 0x00, marker];
        let mut wire = BytesMut::new();
        encode_bvll6(
            &mut wire,
            &Bvll6::ForwardedNpdu {
                original_source: Bip6Vmac::new([0x00, 0x00, 0x09]),
                original_address: Bip6Address::new(
                    Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 9),
                    47808,
                ),
                npdu: &npdu,
            },
        )
        .unwrap();
        wire.to_vec()
    };
    old.send_to(&forwarded(0xA0), to).await.unwrap();
    new.send_to(&forwarded(0xB0), to).await.unwrap();
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.last(), Some(&0xB0), "only the new BBMD's");
    transport.stop().await.unwrap();
}
