//! Strict admission over the existing wire and notification channels.

use super::*;
use crate::client::cov_notification_tests::cov_notification;
use bacnet_types::enums::{ErrorClass, ErrorCode};

const SINGLE: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;

fn payload(notice: &COVNotificationRequest) -> Bytes {
    let mut encoded = BytesMut::new();
    notice.encode(&mut encoded).unwrap();
    encoded.freeze()
}

async fn expect_unknown(
    pair: &mut Pair,
    service: ConfirmedServiceChoice,
    id: u8,
    data: Bytes,
    route: Option<(u16, &[u8])>,
) {
    pair.send(confirmed(id, service, data), route).await;
    let (npdu, reply) = pair.answer().await;
    assert_eq!(
        npdu.destination
            .as_ref()
            .map(|dst| (dst.network, dst.mac_address.as_slice())),
        route
    );
    let Apdu::Error(error) = reply else {
        panic!("expected service Error, got {reply:?}")
    };
    assert_eq!((error.invoke_id, error.service_choice), (id, service));
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::SERVICES, ErrorCode::UNKNOWN_SUBSCRIPTION)
    );
    pair.assert_quiet().await;
}

#[tokio::test(start_paused = true)]
async fn strict_admission_checks_the_wire_source_process_object_and_device() {
    let options = ClientOptions {
        confirmed_cov_subscription_limit: std::num::NonZeroUsize::new(2),
        ..ClientOptions::default()
    };
    let mut pair = pair(options).await;
    let notice = cov_notification(6);
    let object = notice.monitored_object_identifier;
    let remote: &[u8] = &[17];
    let route = Some((55, remote));
    let _lease = pair
        .client
        .expect_confirmed_cov(Station::routed(&PEER, 55, remote), 6, object, Some(100))
        .unwrap();
    let other = pair
        .client
        .expect_confirmed_cov(Station::routed(&[0x99], 56, remote), 6, object, None)
        .unwrap();
    pair.assert_quiet().await;
    expect_unknown(&mut pair, SINGLE, 1, payload(&notice), None).await;
    expect_unknown(&mut pair, SINGLE, 2, payload(&notice), Some((55, &[18]))).await;
    expect_unknown(&mut pair, SINGLE, 3, payload(&notice), Some((56, remote))).await;
    expect_unknown(&mut pair, SINGLE, 4, payload(&cov_notification(7)), route).await;
    let mut wrong = notice.clone();
    wrong.monitored_object_identifier = oid(ObjectType::ANALOG_INPUT, 2);
    expect_unknown(&mut pair, SINGLE, 5, payload(&wrong), route).await;
    wrong = notice.clone();
    wrong.initiating_device_identifier = oid(ObjectType::DEVICE, 101);
    expect_unknown(&mut pair, SINGLE, 6, payload(&wrong), route).await;
    let multiple_service = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
    expect_unknown(
        &mut pair,
        multiple_service,
        7,
        encoded(&multiple(6, false)),
        route,
    )
    .await;
    other.close();
    let _any_router = pair
        .client
        .expect_confirmed_cov(Station::remote(56, remote), 6, object, None)
        .unwrap();
    for (id, network) in [(8, 55), (9, 56)] {
        pair.send(
            confirmed(id, SINGLE, payload(&notice)),
            Some((network, remote)),
        )
        .await;
        let (npdu, reply) = pair.answer().await;
        assert_eq!(npdu.destination.unwrap().network, network);
        assert!(
            matches!(reply, Apdu::SimpleAck(ack) if ack.invoke_id == id && ack.service_choice == SINGLE)
        );
        assert_eq!(
            pair.next_event().await.into_object().unwrap().notification,
            notice
        );
    }
    pair.stop().await;
}

#[tokio::test(start_paused = true)]
async fn registry_capacity_and_close_are_local_and_lease_qualified() {
    let options = ClientOptions {
        confirmed_cov_subscription_limit: std::num::NonZeroUsize::new(1),
        ..ClientOptions::default()
    };
    let mut pair = pair(options).await;
    let notice = cov_notification(3);
    let register = |process| {
        pair.client.expect_confirmed_cov(
            Station::local(&PEER),
            process,
            notice.monitored_object_identifier,
            None,
        )
    };
    let old = register(3).unwrap();
    assert!(register(3).is_err());
    assert!(register(4).is_err());
    old.close();
    let current = register(3).unwrap();
    drop(old);
    pair.assert_quiet().await;
    pair.send(confirmed(0x40, SINGLE, payload(&notice)), None)
        .await;
    assert!(matches!(pair.answer().await.1, Apdu::SimpleAck(ack) if ack.invoke_id == 0x40));
    assert_eq!(
        pair.next_event().await.into_object().unwrap().notification,
        notice
    );
    drop(current);
    expect_unknown(&mut pair, SINGLE, 0x41, payload(&notice), None).await;
    pair.send(
        unconfirmed(
            UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            payload(&notice),
        ),
        None,
    )
    .await;
    assert_eq!(
        pair.next_event().await.into_object().unwrap().notification,
        notice
    );
    pair.assert_quiet().await;
    pair.stop().await;
}
