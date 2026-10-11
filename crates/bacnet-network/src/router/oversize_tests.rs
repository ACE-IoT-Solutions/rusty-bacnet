use super::ingress_harness::*;
use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::enums::RejectMessageReason;

#[tokio::test]
async fn routed_npdu_over_egress_limit_is_rejected_and_counted() {
    let (ports, mut peers) = fixture_with_limits(vec![None, Some(8)]);
    let (mut router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;

    peers[0]
        .tx
        .send(incoming(Some(address(network(1), &[9])), 42))
        .await
        .unwrap();

    let SendRequest::Unicast {
        npdu,
        mac,
        data_attributes,
    } = wire(&mut peers[0]).await
    else {
        panic!("oversize routed NPDU must produce a unicast reject")
    };
    let reject = decode_npdu(npdu).unwrap();
    assert_eq!(mac.as_slice(), &[0xA2]);
    assert_eq!(
        reject.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
    );
    assert_eq!(reject.destination, Some(address(300, &[0x55])));
    assert_eq!(
        reject.payload.as_ref(),
        [
            RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
            0,
            network(1) as u8,
        ]
    );
    assert_eq!(data_attributes, incoming(None, 42).data_attributes);
    assert!(peers[1].wire.try_recv().is_err());
    assert_eq!(router.oversize_drops(), 1);

    router.stop().await;
}

#[tokio::test]
async fn routed_npdu_at_egress_limit_is_forwarded_without_counting() {
    let (ports, mut peers) = fixture_with_limits(vec![None, Some(9)]);
    let (mut router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;

    peers[0]
        .tx
        .send(incoming(Some(address(network(1), &[9])), 42))
        .await
        .unwrap();

    let SendRequest::Unicast { npdu, mac, .. } = wire(&mut peers[1]).await else {
        panic!("NPDU at the limit must be forwarded")
    };
    assert_eq!(mac.as_slice(), &[9]);
    assert_eq!(npdu.len(), 9);
    assert_eq!(router.oversize_drops(), 0);
    assert!(peers[0].wire.try_recv().is_err());

    router.stop().await;
}
