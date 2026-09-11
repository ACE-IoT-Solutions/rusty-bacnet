use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

fn analog_object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

async fn receive_routed_subscribe(
    router_rx: &mut tokio::sync::mpsc::Receiver<ReceivedNpdu>,
    client_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
) -> (u8, SubscribeCOVRequest) {
    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for SubscribeCOV")
        .expect("router channel closed");
    assert_eq!(&received.source_mac[..], client_mac);

    let npdu = decode_npdu(received.npdu).unwrap();
    let destination = npdu.destination.expect("routed NPDU destination");
    assert_eq!(destination.network, remote_network);
    assert_eq!(&destination.mac_address[..], remote_mac);

    let decoded = decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(request) = decoded else {
        panic!("expected confirmed request, got {decoded:?}");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    (
        request.invoke_id,
        SubscribeCOVRequest::decode(&request.service_request).unwrap(),
    )
}

async fn send_routed_simple_ack<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
    invoke_id: u8,
) {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(remote_mac),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    transport.send_unicast(&npdu, client_mac).await.unwrap();
}

fn routed_notification(
    router_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
    process_id: u32,
    object_id: ObjectIdentifier,
    time_remaining: u32,
) -> ReceivedCOVNotification {
    ReceivedCOVNotification::new(
        COVNotificationRequest {
            subscriber_process_identifier: process_id,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 6001).unwrap(),
            monitored_object_identifier: object_id,
            time_remaining,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
                priority: None,
            }],
        },
        router_mac,
        &Some(NpduAddress {
            network: remote_network,
            mac_address: MacAddr::from_slice(remote_mac),
        }),
        COVNotificationDelivery::Unconfirmed,
    )
}

#[tokio::test]
async fn routed_managed_subscription_renews_via_existing_routed_request_path() {
    let client_mac = vec![0x61];
    let router_mac = vec![0x62];
    let remote_network = 2001;
    let remote_mac = vec![0x0a, 0x0b];
    let object_id = analog_object(7);
    let process_id = 7007;
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    let start_client = Arc::clone(&client);
    let start_router = router_mac.clone();
    let start_remote = remote_mac.clone();
    let start = tokio::spawn(async move {
        start_client
            .manage_cov_subscription_routed(
                &start_router,
                remote_network,
                &start_remote,
                process_id,
                object_id,
                false,
                10,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_secs(2)),
            )
            .await
    });

    let (initial_invoke_id, initial) =
        receive_routed_subscribe(&mut router_rx, &client_mac, remote_network, &remote_mac).await;
    assert_eq!(initial.subscriber_process_identifier, process_id);
    assert_eq!(initial.lifetime, Some(10));
    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        initial_invoke_id,
    )
    .await;

    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();
    client
        .cov_tx
        .send(routed_notification(
            &router_mac,
            remote_network,
            &remote_mac,
            process_id,
            object_id,
            1,
        ))
        .unwrap();

    assert!(matches!(
        timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap(),
        ManagedCOVSubscriptionEvent::NotificationObserved {
            time_remaining: 1,
            renew_after
        } if renew_after.is_zero()
    ));
    assert_eq!(
        timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap(),
        ManagedCOVSubscriptionEvent::ImpendingExpiry { time_remaining: 1 }
    );

    let (renew_invoke_id, renewal) =
        receive_routed_subscribe(&mut router_rx, &client_mac, remote_network, &remote_mac).await;
    assert_eq!(renewal.subscriber_process_identifier, process_id);
    assert_eq!(renewal.lifetime, Some(10));
    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        renew_invoke_id,
    )
    .await;
    assert!(matches!(
        timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap(),
        ManagedCOVSubscriptionEvent::Renewed {
            requested_lifetime: 10,
            ..
        }
    ));

    managed.stop().await;
    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed task retained client after stop"),
    };
    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

#[test]
fn routed_matching_requires_router_and_npdu_source_identity() {
    let object_id = analog_object(8);
    let target = ManagedCOVTarget::Routed {
        router_mac: MacAddr::from_slice(&[0x71]),
        dest_network: 3001,
        dest_mac: MacAddr::from_slice(&[0x72]),
    };
    let matching = routed_notification(&[0x71], 3001, &[0x72], 8, object_id, 10);
    assert!(matches_managed_cov_notification(
        &matching, &target, 8, object_id
    ));

    let wrong_source = routed_notification(&[0x71], 3001, &[0x73], 8, object_id, 10);
    assert!(!matches_managed_cov_notification(
        &wrong_source,
        &target,
        8,
        object_id
    ));
}
