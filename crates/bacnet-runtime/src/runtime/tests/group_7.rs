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

async fn install_cov_path_test_spec(
    runtime: &BacnetRuntime,
    device: DeviceKey,
    path: DevicePath,
) -> ObservationKey {
    runtime
        .inner
        .device_index
        .write()
        .await
        .upsert(DeviceObservation {
            key: device,
            path,
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    let key = ObservationKey {
        device,
        object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
        object_instance: 17,
        subscriber_process_id: 9_001,
    };
    let plan = ObservationPlan {
        revision: 1,
        subscriptions: vec![ObservationSpec {
            key,
            confirmed: false,
            lifetime_seconds: 60,
            renewal_margin: Duration::from_secs(10),
            suppress_poll_when_fresh: true,
        }],
    };
    let changes = runtime.inner.observations.read().await.diff(plan).unwrap();
    runtime.inner.observations.write().await.commit(changes);
    while !runtime
        .next_events(16, Duration::ZERO)
        .await
        .events
        .is_empty()
    {}
    key
}

fn cov_path_test_notification(
    device_instance: u32,
    source_mac: &[u8],
    source_network: Option<u16>,
    source_address: Option<&[u8]>,
) -> ReceivedCOVNotification {
    ReceivedCOVNotification {
        notification: COVNotificationRequest {
            subscriber_process_identifier: 9_001,
            initiating_device_identifier: ObjectIdentifier::new(
                ObjectType::DEVICE,
                device_instance,
            )
            .unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17)
                .unwrap(),
            time_remaining: 47,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x41, 0xfc, 0x00, 0x00],
                priority: None,
            }],
        },
        source_mac: MacAddr::from_slice(source_mac),
        source_network,
        source_address: source_address.map(MacAddr::from_slice),
        delivery: COVNotificationDelivery::Unconfirmed,
    }
}

#[tokio::test]
async fn managed_direct_cov_rejects_second_socket_before_cache_or_event_update() {
    let attachment_id = AttachmentId::from(92);
    let device = DeviceKey {
        attachment_id,
        device_instance: 4_190_092,
    };
    let expected_mac = [127, 0, 0, 1, 0xba, 0xc1];
    let second_socket_mac = [127, 0, 0, 1, 0xba, 0xc2];
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(92, "managed-direct-path-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let key = install_cov_path_test_spec(
        &runtime,
        device,
        DevicePath::Direct {
            mac: expected_mac.to_vec(),
        },
    )
    .await;

    runtime
        .handle_cov_notification(
            attachment_id,
            cov_path_test_notification(device.device_instance, &second_socket_mac, None, None),
        )
        .await;
    assert!(runtime.inner.cov_last.lock().await.get(&key).is_none());
    assert_eq!(runtime.inner.values.read().await.len(), 0);
    let rejected = runtime.next_events(16, Duration::ZERO).await;
    assert!(rejected
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::UnsolicitedCovNotification { .. })));
    assert!(!rejected
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::CovNotification { .. })));

    runtime
        .handle_cov_notification(
            attachment_id,
            cov_path_test_notification(device.device_instance, &expected_mac, None, None),
        )
        .await;
    assert!(runtime.inner.cov_last.lock().await.contains_key(&key));
    assert_eq!(runtime.inner.values.read().await.len(), 1);
    assert!(runtime
        .next_events(16, Duration::ZERO)
        .await
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::CovNotification { key: event_key, .. } if event_key == key)));
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn managed_routed_cov_requires_ingress_dnet_and_dadr_before_update() {
    let attachment_id = AttachmentId::from(93);
    let device = DeviceKey {
        attachment_id,
        device_instance: 4_190_093,
    };
    let ingress_mac = [127, 0, 0, 1, 0xba, 0xc3];
    let dnet = 2200;
    let dadr = [0x00, 0x05];
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(93, "managed-routed-path-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let key = install_cov_path_test_spec(
        &runtime,
        device,
        DevicePath::Routed {
            ingress_mac: ingress_mac.to_vec(),
            dnet,
            dadr: dadr.to_vec(),
        },
    )
    .await;

    for (source_network, source_address) in [
        (Some(dnet + 1), Some(dadr.as_slice())),
        (Some(dnet), Some([0x00, 0x06].as_slice())),
    ] {
        runtime
            .handle_cov_notification(
                attachment_id,
                cov_path_test_notification(
                    device.device_instance,
                    &ingress_mac,
                    source_network,
                    source_address,
                ),
            )
            .await;
    }
    assert!(runtime.inner.cov_last.lock().await.get(&key).is_none());
    assert_eq!(runtime.inner.values.read().await.len(), 0);
    let rejected = runtime.next_events(16, Duration::ZERO).await;
    assert_eq!(
        rejected
            .events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::UnsolicitedCovNotification { .. }))
            .count(),
        2
    );
    assert!(!rejected
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::CovNotification { .. })));

    runtime
        .handle_cov_notification(
            attachment_id,
            cov_path_test_notification(
                device.device_instance,
                &ingress_mac,
                Some(dnet),
                Some(&dadr),
            ),
        )
        .await;
    assert!(runtime.inner.cov_last.lock().await.contains_key(&key));
    assert_eq!(runtime.inner.values.read().await.len(), 1);
    assert!(runtime
        .next_events(16, Duration::ZERO)
        .await
        .events
        .iter()
        .any(|event| matches!(event.kind, EventKind::CovNotification { key: event_key, .. } if event_key == key)));
    runtime.stop().await.unwrap();
}
