#[tokio::test]
async fn attachment_removal_drops_cov_registry_tasks_and_reports_receiver_lag() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let server_task = tokio::spawn(async move {
        let message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let request = match apdu::decode_apdu(message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected SubscribeCOV, got {other:?}"),
        };
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &message.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();
        server.stop().await.unwrap();
    });
    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "removed-cov-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    runtime
        .inner
        .device_index
        .write()
        .await
        .upsert(DeviceObservation {
            key: device,
            path: DevicePath::Direct { mac: server_mac },
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    runtime
        .apply_observation_plan(ObservationPlan {
            revision: 1,
            subscriptions: vec![ObservationSpec {
                key: ObservationKey {
                    device,
                    object_type: 0,
                    object_instance: 1,
                    subscriber_process_id: 88,
                },
                confirmed: false,
                lifetime_seconds: 60,
                renewal_margin: Duration::from_secs(10),
                suppress_poll_when_fresh: false,
            }],
        })
        .await
        .unwrap();
    server_task.await.unwrap();
    runtime.record_cov_lag(attachment_id, 7).await;
    let events = runtime.next_events(16, Duration::ZERO).await;
    assert!(events.events.iter().any(|event| {
        event.attachment_id == Some(attachment_id)
            && event.kind == EventKind::CovNotificationLagged { skipped: 7 }
    }));
    runtime.reconcile(2, Vec::new()).await.unwrap();
    assert_eq!(runtime.health().await.observation_count, 0);
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.health().await.task_count != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn unsolicited_cov_event_preserves_payload_source_and_attachment() {
    let attachment_id = AttachmentId::from(91);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(91, "passive-cov-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let initiating_device_identifier =
        ObjectIdentifier::new(ObjectType::DEVICE, 4_190_091).unwrap();
    let monitored_object_identifier = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17).unwrap();
    let source_mac = MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xc0]);
    let source_address = MacAddr::from_slice(&[0, 7, 0xaa]);
    runtime
        .handle_cov_notification(
            attachment_id,
            ReceivedCOVNotification {
                notification: COVNotificationRequest {
                    subscriber_process_identifier: 9_001,
                    initiating_device_identifier,
                    monitored_object_identifier,
                    time_remaining: 47,
                    list_of_values: vec![
                        BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            value: vec![0x44, 0x41, 0xfc, 0x00, 0x00],
                            priority: None,
                        },
                        BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::DESCRIPTION,
                            property_array_index: Some(3),
                            value: vec![0x71, 0x00],
                            priority: Some(8),
                        },
                    ],
                },
                source_mac: source_mac.clone(),
                source_network: Some(2001),
                source_address: Some(source_address.clone()),
                delivery: COVNotificationDelivery::Confirmed,
            },
        )
        .await;

    let batch = runtime.next_events(16, Duration::ZERO).await;
    assert!(!batch
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::CovNotification { .. })));
    let event = batch
        .events
        .iter()
        .find(|event| matches!(event.kind, EventKind::UnsolicitedCovNotification { .. }))
        .expect("unsolicited COV event");
    assert_eq!(event.attachment_id, Some(attachment_id));
    let EventKind::UnsolicitedCovNotification { notification } = &event.kind else {
        unreachable!();
    };
    assert_eq!(notification.subscriber_process_identifier, 9_001);
    assert_eq!(
        notification.initiating_device_identifier,
        initiating_device_identifier
    );
    assert_eq!(
        notification.monitored_object_identifier,
        monitored_object_identifier
    );
    assert_eq!(notification.time_remaining, 47);
    assert_eq!(notification.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(notification.source_mac, source_mac);
    assert_eq!(notification.source_network, Some(2001));
    assert_eq!(notification.source_address, Some(source_address));
    assert_eq!(notification.values.len(), 2);
    assert_eq!(notification.values[0].property_id, 85);
    assert_eq!(
        notification.values[0].raw_value,
        vec![0x44, 0x41, 0xfc, 0x00, 0x00]
    );
    assert_eq!(notification.values[1].property_id, 28);
    assert_eq!(notification.values[1].array_index, Some(3));
    assert_eq!(notification.values[1].raw_value, vec![0x71, 0x00]);
    assert_eq!(notification.values[1].priority, Some(8));
    runtime.stop().await.unwrap();
}

