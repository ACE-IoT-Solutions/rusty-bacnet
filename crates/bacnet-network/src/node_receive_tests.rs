//! The non-router [`NetworkLayer`] on the router core's node (#1795): what
//! carries a DNET naming one network reaches neither receiver, network
//! messages included (Clauses 6.5.2.1 and 6.5.2.2), and every drop is
//! totalled in [`NetworkLayer::counters`].

use bacnet_npdu::NetworkMessageType;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;

use crate::layer::{Counters, NetworkLayer};
use crate::loopback_fixture::{recv, wire, APDU};

#[tokio::test]
async fn npdus_for_another_network_reach_neither_receiver_and_are_counted() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut controls = network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start().await.unwrap();
    let control = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);

    // For station 0xD0 on network 3, and network 3's broadcast: an APDU and
    // a network message each.
    let routed = [
        wire(Some((3, 1)), None, None),
        wire(Some((3, 0)), None, None),
        wire(Some((3, 1)), None, control),
        wire(Some((3, 0)), None, control),
    ];
    for bytes in &routed {
        peer.send_unicast(bytes, &[0x01]).await.unwrap();
    }
    // Then a local APDU and a local network message, and a global one each.
    for bytes in [
        wire(None, None, None),
        wire(None, None, control),
        wire(Some((0xFFFF, 0)), None, None),
        wire(Some((0xFFFF, 0)), None, control),
    ] {
        peer.send_broadcast(&bytes).await.unwrap();
    }

    let local = recv(&mut apdus).await;
    assert_eq!(local.apdu, APDU[..]);
    assert!(
        local.is_group && !local.global_broadcast,
        "the local APDU first"
    );
    let global = recv(&mut apdus).await;
    assert!(global.is_group && global.global_broadcast);

    let local = recv(&mut controls).await;
    assert_eq!(local.npdu.destination, None, "the local message first");
    assert_eq!(local.npdu.payload[..], [0x0B, 0xB8]);
    let global = recv(&mut controls).await;
    assert_eq!(global.npdu.destination.map(|d| d.network), Some(0xFFFF));

    let expected = Counters {
        routed_drops: routed.len() as u64,
        ..Counters::default()
    };
    assert_eq!(network.counters(), expected);

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn malformed_npdus_are_counted() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut apdus = network.start().await.unwrap();

    // Cut short, a version other than 1, and an SNET of 0.
    for bytes in [vec![0x01], vec![0x02, 0x00, 0x10, 0x08], {
        let mut zero_snet = wire(None, Some((1, 1)), None);
        zero_snet[2..4].copy_from_slice(&[0, 0]);
        zero_snet
    }] {
        peer.send_unicast(&bytes, &[0x01]).await.unwrap();
    }
    peer.send_unicast(&wire(None, None, None), &[0x01])
        .await
        .unwrap();
    assert_eq!(recv(&mut apdus).await.apdu, APDU[..], "only the good one");

    let expected = Counters {
        malformed: 3,
        ..Counters::default()
    };
    assert_eq!(network.counters(), expected);

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}
