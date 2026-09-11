use super::*;

use bacnet_network::observer::{ApduDecode, ApduDirection, ApduObserver, DecodeStage};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};

#[test]
fn observer_is_disabled_by_default_and_available_on_native_builders() {
    assert!(BACnetServer::<LoopbackTransport>::generic_builder()
        .apdu_observer
        .is_none());

    let (generic_observer, _) = ApduObserver::new(4);
    assert!(BACnetServer::<LoopbackTransport>::generic_builder()
        .apdu_observer(generic_observer)
        .apdu_observer
        .is_some());

    let (bip_observer, _) = ApduObserver::new(4);
    assert!(BACnetServer::bip_builder()
        .apdu_observer(bip_observer)
        .apdu_observer
        .is_some());
}

#[tokio::test]
async fn generic_builder_observer_is_passive_bounded_and_closes_on_stop() {
    let server_mac = vec![0x51];
    let peer_mac = vec![0x52];
    let (server_transport, mut peer_transport) =
        LoopbackTransport::pair(server_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let (observer, mut events) = ApduObserver::new(1);
    let mut server = BACnetServer::generic_builder()
        .transport(server_transport)
        .apdu_observer(observer)
        .build()
        .await
        .unwrap();

    peer_transport
        .send_unicast(&[0xff], &server_mac)
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("observer delivery timed out")
        .expect("observer closed before delivery");
    assert_eq!(event.direction, ApduDirection::Inbound);
    assert_eq!(event.immediate_peer, MacAddr::from_slice(&peer_mac));
    assert!(matches!(
        event.decode,
        ApduDecode::Error(ref error) if error.stage == DecodeStage::Npdu
    ));
    assert!(timeout(Duration::from_millis(50), peer_rx.recv())
        .await
        .is_err());

    for raw in [0xfe, 0xfd, 0xfc] {
        peer_transport
            .send_unicast(&[raw], &server_mac)
            .await
            .unwrap();
    }
    tokio::task::yield_now().await;
    assert!(matches!(
        events.recv().await,
        Err(broadcast::error::RecvError::Lagged(2))
    ));
    assert_eq!(events.recv().await.unwrap().raw_npdu.as_ref(), &[0xfc]);

    server.stop().await.unwrap();
    assert!(matches!(
        events.recv().await,
        Err(broadcast::error::RecvError::Closed)
    ));
    peer_transport.stop().await.unwrap();
}
