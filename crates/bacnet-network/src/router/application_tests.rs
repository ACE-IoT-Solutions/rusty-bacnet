//! The router's application port: what a stack sends becomes the router's
//! own request, routed by its table; what the router delivers comes up with
//! its origin as SNET and SADR; refusals keep their reason (#1808).

use std::io;

use bacnet_npdu::{Mac, NpduLimits};
use bacnet_router_core::PortId;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use super::super::priority_harness::*;
use super::{OriginateError, OriginateFailure, RouterApplicationPort, SourceSelection};
use crate::layer::{AdmissionReceiver, ReceivedApdu};

const READ_PROPERTY: [u8; 4] = [0x00, 0x05, 0x01, 0x0C];

/// A ReadProperty to network 3000, station 5, expecting a reply, at
/// Urgent priority, as a stack writes it.
fn routed_request() -> Vec<u8> {
    [&[1, 0x25, 0x0B, 0xB8, 1, 5, 255][..], &READ_PROPERTY].concat()
}

#[tokio::test]
async fn a_routed_send_is_originated_by_the_table() {
    let (mut router, apdus, mut peers) = start(2).await;
    let now = router.now();
    assert!(
        router
            .table()
            .add_learned(3000, PortId(1), Mac::from_array([7]), now)
    );
    let port = router.application_port(apdus);
    // The MAC the stack sends to is ignored: the table picks [7].
    port.send_unicast(&routed_request(), &[0xEE]).await.unwrap();
    let sent = sending(&mut peers[1]).await;
    assert_eq!(sent.to, Some(vec![7]));
    // Its expecting-reply bit and priority kept, the hop count full, no
    // SNET: from the egress port's address.
    assert_eq!(sent.npdu, routed_request());
    sent.complete.send(Ok(())).unwrap();

    // With the source set to port 0: SNET 1000, SADR the router's MAC there.
    let port = port.with_source(SourceSelection::Port(PortId(0)));
    port.send_broadcast(&routed_request()).await.unwrap();
    let sent = sending(&mut peers[1]).await;
    assert_eq!(sent.to, Some(vec![7]), "a unicast all the same");
    assert_eq!(
        sent.npdu,
        [
            &[1, 0x2D, 0x0B, 0xB8, 1, 5, 0x03, 0xE8, 1, 1, 255][..],
            &READ_PROPERTY
        ]
        .concat()
    );
    sent.complete.send(Ok(())).unwrap();
    assert!(peers[0].sent.try_recv().is_err(), "nothing on port 0");
    assert_eq!(router.counters().originated, 2);
    router.stop().await.unwrap();
}

/// A device on a directly attached network: its network as DNET.
#[tokio::test]
async fn a_device_on_an_attached_network_is_reached_directly() {
    let (mut router, apdus, mut peers) = start(2).await;
    let port = router.application_port(apdus);
    let to_2000 = [&[1, 0x24, 0x07, 0xD0, 1, 5, 255][..], &READ_PROPERTY].concat();
    port.send_unicast(&to_2000, &[1]).await.unwrap();
    let sent = sending(&mut peers[1]).await;
    assert_eq!(sent.to, Some(vec![5]));
    assert_eq!(sent.npdu, [&[1, 0x04][..], &READ_PROPERTY].concat());
    sent.complete.send(Ok(())).unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn deliveries_come_up_with_their_origin() {
    let (mut router, apdus, peers) = start(2).await;
    let mut port = router.application_port(apdus);
    let mut frames = port.start().await.unwrap();
    let complex_ack = [0x30, 0x05, 0x0C];

    // Relayed by a router on network 2000: its SNET and SADR kept.
    receive(
        &peers[1],
        [&[1, 0x08, 0x0B, 0xB8, 1, 5][..], &complex_ack].concat(),
        0,
    )
    .await;
    let up = frames.recv().await.unwrap();
    assert_eq!(
        up.npdu.as_ref(),
        [&[1, 0x08, 0x0B, 0xB8, 1, 5][..], &complex_ack].concat()
    );
    assert_eq!(up.source_mac.as_slice(), [7]);
    assert!(!up.link_layer_group);
    assert!(up.reply_tx.is_none() && up.direct_response.is_none());

    // From station 7 on network 1000 itself: SNET 1000, SADR 7.
    receive(&peers[0], [&[1, 0][..], &complex_ack].concat(), 0).await;
    let up = frames.recv().await.unwrap();
    assert_eq!(
        up.npdu.as_ref(),
        [&[1, 0x08, 0x03, 0xE8, 1, 7][..], &complex_ack].concat()
    );

    // A global Who-Is stays global; a local one comes up as a group.
    let who_is = [0x10, 0x08];
    receive(
        &peers[0],
        [&[1, 0x20, 0xFF, 0xFF, 0, 255][..], &who_is].concat(),
        0,
    )
    .await;
    let up = frames.recv().await.unwrap();
    assert_eq!(
        up.npdu.as_ref(),
        [
            &[1, 0x28, 0xFF, 0xFF, 0, 0x03, 0xE8, 1, 7, 255][..],
            &who_is
        ]
        .concat()
    );
    assert!(!up.link_layer_group, "DNET 0xFFFF says it");
    peers[1]
        .frames
        .send(bacnet_transport::port::ReceivedNpdu::unverified(
            bytes::Bytes::from([&[1, 0][..], &who_is].concat()),
            bacnet_types::MacAddr::from_slice(&[9]),
            true,
            vec![],
            None,
        ))
        .await
        .unwrap();
    let up = frames.recv().await.unwrap();
    assert_eq!(
        up.npdu.as_ref(),
        [&[1, 0x08, 0x07, 0xD0, 1, 9][..], &who_is].concat()
    );
    assert!(up.link_layer_group);

    // Stopping the router closes its local queue, which ends the port's.
    router.stop().await.unwrap();
    assert!(frames.recv().await.is_none());
    port.stop().await.unwrap();
}

#[tokio::test]
async fn refusals_keep_their_reason() {
    let (mut router, apdus, mut peers) = start(2).await;
    let port = router.application_port(apdus);

    // No DNET: a local send, on a link the application lacks.
    let local = [&[1, 0x04][..], &READ_PROPERTY].concat();
    assert!(matches!(
        port.send_unicast(&local, &[5]).await,
        Err(Error::Encoding(_))
    ));
    // A network message, a router query.
    assert!(matches!(
        port.send_broadcast(&[1, 0x80, 0x00]).await,
        Err(Error::Encoding(_))
    ));
    // Not an NPDU at all.
    assert!(matches!(
        port.send_unicast(&[2, 0], &[5]).await,
        Err(Error::Encoding(_))
    ));

    // No route: refused as the router refuses it, and searched for.
    let error = port
        .send_unicast(&routed_request(), &[1])
        .await
        .unwrap_err();
    let Error::Transport(io) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(io.kind(), io::ErrorKind::NetworkUnreachable);
    assert_eq!(
        OriginateFailure::from_error(&error),
        Some(OriginateFailure::Refused(OriginateError::NoRoute))
    );
    for peer in &mut peers {
        let search = sending(peer).await;
        assert_eq!(search.npdu, [1, 0x80, 0x00, 0x0B, 0xB8]);
        search.complete.send(Ok(())).unwrap();
    }

    // Too long for the egress port: the length refusal.
    let long = [&[1, 0x24, 0x07, 0xD0, 1, 5, 255][..], &[0x10; 1500][..]].concat();
    let error = port.send_unicast(&long, &[1]).await.unwrap_err();
    // Delivered on network 2000 itself, the NPDU loses its DNET.
    assert!(
        matches!(
            error,
            Error::SendTooLong {
                length: 1502,
                limit: 1497
            }
        ),
        "{error:?}"
    );
    assert_eq!(
        OriginateFailure::from_error(&Error::Encoding("x".into())),
        None
    );

    router.stop().await.unwrap();
    let error = port
        .send_unicast(&routed_request(), &[1])
        .await
        .unwrap_err();
    assert_eq!(
        OriginateFailure::from_error(&error),
        Some(OriginateFailure::Stopped)
    );
}

#[tokio::test]
async fn limits_and_address() {
    let (mut router, apdus, _peers) = start(2).await;
    let port = router.application_port(apdus);
    assert_eq!(port.local_mac(), [1]);
    assert_eq!(port.max_npdu(), NpduLimits::both(1497));
    assert_eq!(port.local_receive_apdu_capacity(), 1476);
    assert_eq!(port.egress_apdu_limit(), 1476);
    let mut port_mstp = port.with_max_npdu(NpduLimits::both(501));
    assert_eq!(port_mstp.egress_apdu_limit(), 480);
    port_mstp.start().await.unwrap();
    assert!(
        port_mstp.start().await.is_err(),
        "the router's receiver is taken once"
    );
    port_mstp.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// The tracked receiver feeds a port too.
#[allow(dead_code)]
fn tracked(router: &super::BACnetRouter, apdus: AdmissionReceiver<ReceivedApdu>) {
    let _: RouterApplicationPort = router.application_port(apdus);
}
