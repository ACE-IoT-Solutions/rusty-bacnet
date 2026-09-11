use super::*;

use bacnet_network::observer::{ApduDecode, ApduDirection, ApduObserver};
use bacnet_transport::loopback::LoopbackTransport;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn generic_builder_observer_is_passive_bounded_and_closes_on_stop() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let (observer, mut events) = ApduObserver::new(4);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_observer(observer)
        .build()
        .await
        .unwrap();

    client.who_is(None, None).await.unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("observer delivery timed out")
        .expect("observer closed before delivery");

    assert_eq!(event.direction, ApduDirection::Outbound);
    assert!(matches!(
        event.decode,
        ApduDecode::Decoded(ref summary)
            if summary.pdu_type == "unconfirmed_request"
                && summary.service_choice == Some(UnconfirmedServiceChoice::WHO_IS.to_raw() as u32)
    ));

    client.stop().await.unwrap();
    assert!(matches!(
        events.recv().await,
        Err(broadcast::error::RecvError::Closed)
    ));
}

#[test]
fn observer_is_disabled_in_default_client_options() {
    assert!(ClientOptions::default().apdu_observer.is_none());
}
