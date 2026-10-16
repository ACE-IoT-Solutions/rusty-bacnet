//! Explicit route announcements after one data-link port recovers.

use super::ingress_harness::{drain_announcements, fixture, launch, network, wire};
use super::{RouterOptions, SendRequest};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::enums::NetworkMessageType;

#[tokio::test]
async fn recovery_reannounces_only_on_the_selected_port() {
    let (ports, mut peers) = fixture(3);
    let (mut router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;

    router.reannounce_port(1).unwrap();

    assert!(peers[0].wire.try_recv().is_err());
    assert!(peers[2].wire.try_recv().is_err());

    let mut number = network(1).to_be_bytes().to_vec();
    number.push(1);
    let beyond: Vec<u8> = [network(0), network(2)]
        .into_iter()
        .flat_map(u16::to_be_bytes)
        .collect();
    for (message_type, data) in [
        (NetworkMessageType::NETWORK_NUMBER_IS, number),
        (NetworkMessageType::I_AM_ROUTER_TO_NETWORK, beyond),
    ] {
        let SendRequest::Broadcast {
            npdu,
            data_attributes,
        } = wire(&mut peers[1]).await
        else {
            panic!("expected recovery announcement")
        };
        assert!(data_attributes.is_empty());
        let npdu = decode_npdu(npdu).unwrap();
        assert_eq!(npdu.message_type, Some(message_type.to_raw()));
        assert_eq!(npdu.payload[..], data[..]);
    }

    assert!(router.reannounce_port(3).is_err());
    router.stop().await;
    assert!(router.reannounce_port(1).is_err());
}
