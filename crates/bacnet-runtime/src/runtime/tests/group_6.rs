#[tokio::test]
async fn routed_cov_plan_and_notification_preserve_remote_path() {
    let mut router = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut router_rx = router.start().await.unwrap();
    let router_mac = router.local_mac().to_vec();
    let remote_network = 2200;
    let remote_mac = vec![0x00, 0x05];
    let expected_remote_mac = remote_mac.clone();
    let router_task = tokio::spawn(async move {
        let subscribe_message = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let subscribe_npdu = decode_npdu(subscribe_message.npdu).unwrap();
        let destination = subscribe_npdu.destination.expect("routed COV destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(destination.mac_address.as_ref(), expected_remote_mac);
        let subscribe_request = match apdu::decode_apdu(subscribe_npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected routed SubscribeCOV, got {other:?}"),
        };
        let subscribe = SubscribeCOVRequest::decode(&subscribe_request.service_request).unwrap();
        assert_eq!(subscribe.subscriber_process_identifier, 222);
        assert_eq!(subscribe.lifetime, Some(60));
        assert_eq!(subscribe.issue_confirmed_notifications, Some(false));

        let mut ack_apdu = BytesMut::new();
        encode_apdu(
            &mut ack_apdu,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: subscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        let ack_npdu = Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(&expected_remote_mac),
            }),
            payload: ack_apdu.freeze(),
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &ack_npdu).unwrap();
        router
            .send_unicast(&encoded, subscribe_message.source_mac.as_ref())
            .await
            .unwrap();

        let notification = COVNotificationRequest {
            subscriber_process_identifier: 222,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            time_remaining: 59,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
                priority: None,
            }],
        };
        let mut service = BytesMut::new();
        notification.encode(&mut service);
        let mut notification_apdu = BytesMut::new();
        encode_apdu(
            &mut notification_apdu,
            &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                service_request: service.freeze(),
            }),
        )
        .unwrap();
        let notification_npdu = Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(&expected_remote_mac),
            }),
            payload: notification_apdu.freeze(),
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &notification_npdu).unwrap();
        router
            .send_unicast(&encoded, subscribe_message.source_mac.as_ref())
            .await
            .unwrap();

        let unsubscribe_message = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let unsubscribe_npdu = decode_npdu(unsubscribe_message.npdu).unwrap();
        let destination = unsubscribe_npdu
            .destination
            .expect("routed unsubscribe destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(destination.mac_address.as_ref(), expected_remote_mac);
        let unsubscribe_request = match apdu::decode_apdu(unsubscribe_npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected routed COV unsubscribe, got {other:?}"),
        };
        let unsubscribe =
            SubscribeCOVRequest::decode(&unsubscribe_request.service_request).unwrap();
        assert_eq!(unsubscribe.subscriber_process_identifier, 222);
        assert_eq!(unsubscribe.lifetime, None);
        assert_eq!(unsubscribe.issue_confirmed_notifications, None);
        let mut ack_apdu = BytesMut::new();
        encode_apdu(
            &mut ack_apdu,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: unsubscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        let ack_npdu = Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(&expected_remote_mac),
            }),
            payload: ack_apdu.freeze(),
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &ack_npdu).unwrap();
        router
            .send_unicast(&encoded, unsubscribe_message.source_mac.as_ref())
            .await
            .unwrap();
        router.stop().await.unwrap();
    });

    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 200,
    };
    let path = DevicePath::Routed {
        ingress_mac: router_mac,
        dnet: remote_network,
        dadr: remote_mac,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "routed-cov-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let restored = runtime
        .restore_devices(vec![crate::PersistedDevice {
            key: device,
            path: path.clone(),
            vendor_id: 1,
            max_apdu_length: 1476,
        }])
        .await
        .unwrap();
    assert_eq!(restored.added, 1);
    let key = ObservationKey {
        device,
        object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
        object_instance: 1,
        subscriber_process_id: 222,
    };
    runtime
        .apply_observation_plan(ObservationPlan {
            revision: 1,
            subscriptions: vec![ObservationSpec {
                key,
                confirmed: false,
                lifetime_seconds: 60,
                renewal_margin: Duration::from_secs(10),
                suppress_poll_when_fresh: true,
            }],
        })
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(2), async {
            loop {
                let batch = runtime.next_events(16, Duration::from_millis(20)).await;
                if let Some(event) = batch.events.into_iter().find(|event| {
                    matches!(event.kind, EventKind::CovNotification { key: event_key, .. } if event_key == key)
                }) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(event.attachment_id, Some(attachment_id));
    assert!(matches!(
        event.kind,
        EventKind::CovNotification {
            key: event_key,
            time_remaining: 59,
            ..
        } if event_key == key
    ));
    assert_eq!(
        runtime
            .inner
            .device_index
            .read()
            .await
            .get(device)
            .unwrap()
            .path,
        path
    );
    runtime
        .apply_observation_plan(ObservationPlan {
            revision: 2,
            subscriptions: Vec::new(),
        })
        .await
        .unwrap();
    router_task.await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn real_cov_burst_reports_exact_bounded_event_queue_loss() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        let subscribe_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let subscribe_request = match apdu::decode_apdu(subscribe_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected SubscribeCOV, got {other:?}"),
        };
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: subscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &subscribe_message.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();
        release_rx.await.unwrap();
        for value in 0_u8..8 {
            let notification = COVNotificationRequest {
                subscriber_process_identifier: 333,
                initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 300)
                    .unwrap(),
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                time_remaining: 60,
                list_of_values: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x21, value],
                    priority: None,
                }],
            };
            let mut service = BytesMut::new();
            notification.encode(&mut service);
            let mut encoded = BytesMut::new();
            encode_apdu(
                &mut encoded,
                &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service.freeze(),
                }),
            )
            .unwrap();
            server
                .send_apdu(
                    &encoded,
                    &subscribe_message.source_mac,
                    false,
                    NetworkPriority::NORMAL,
                )
                .await
                .unwrap();
        }
        let unsubscribe_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let unsubscribe_request = match apdu::decode_apdu(unsubscribe_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected COV unsubscribe, got {other:?}"),
        };
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: unsubscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &unsubscribe_message.source_mac,
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
        device_instance: 300,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "cov-overflow-client")],
        event_capacity: 2,
        max_event_batch: 2,
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
                    object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                    object_instance: 1,
                    subscriber_process_id: 333,
                },
                confirmed: false,
                lifetime_seconds: 60,
                renewal_margin: Duration::from_secs(10),
                suppress_poll_when_fresh: false,
            }],
        })
        .await
        .unwrap();
    while !runtime
        .next_events(2, Duration::ZERO)
        .await
        .events
        .is_empty()
    {}
    release_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), async {
        while runtime.health().await.event_lag_count < 6 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let total_lost = runtime.health().await.event_lag_count;
    let retained = runtime.next_events(2, Duration::ZERO).await;
    assert_eq!(retained.events.len(), 2);
    assert!(retained
        .events
        .iter()
        .all(|event| matches!(event.kind, EventKind::CovNotification { .. })));
    let lag = runtime.next_events(2, Duration::ZERO).await;
    assert!(lag.events.iter().any(|event| {
        matches!(
            event.kind,
            EventKind::EventLagged { count, .. } if count == total_lost
        )
    }));
    assert_eq!(total_lost, 6);
    runtime
        .apply_observation_plan(ObservationPlan {
            revision: 2,
            subscriptions: Vec::new(),
        })
        .await
        .unwrap();
    server_task.await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn real_cov_receiver_overflow_reports_lag_event_and_health_total() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (sent_tx, sent_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        let subscribe_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let subscribe_request = match apdu::decode_apdu(subscribe_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected SubscribeCOV, got {other:?}"),
        };
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: subscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &subscribe_message.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();
        release_rx.await.unwrap();
        for value in 0_u8..5 {
            let notification = COVNotificationRequest {
                subscriber_process_identifier: 444,
                initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 400)
                    .unwrap(),
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                time_remaining: 60,
                list_of_values: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x21, value],
                    priority: None,
                }],
            };
            let mut service = BytesMut::new();
            notification.encode(&mut service);
            let mut encoded = BytesMut::new();
            encode_apdu(
                &mut encoded,
                &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service.freeze(),
                }),
            )
            .unwrap();
            server
                .send_apdu(
                    &encoded,
                    &subscribe_message.source_mac,
                    false,
                    NetworkPriority::NORMAL,
                )
                .await
                .unwrap();
        }
        sent_tx.send(()).unwrap();
        let unsubscribe_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let unsubscribe_request = match apdu::decode_apdu(unsubscribe_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected COV unsubscribe, got {other:?}"),
        };
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: unsubscribe_request.invoke_id,
                service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &unsubscribe_message.source_mac,
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
        device_instance: 400,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "cov-receiver-overflow-client")],
        event_capacity: 32,
        max_event_batch: 32,
        cov_channel_capacity: 1,
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
                    object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                    object_instance: 1,
                    subscriber_process_id: 444,
                },
                confirmed: false,
                lifetime_seconds: 60,
                renewal_margin: Duration::from_secs(10),
                suppress_poll_when_fresh: false,
            }],
        })
        .await
        .unwrap();
    while !runtime
        .next_events(32, Duration::ZERO)
        .await
        .events
        .is_empty()
    {}
    let cov_guard = runtime.inner.cov_last.lock().await;
    release_tx.send(()).unwrap();
    sent_rx.await.unwrap();
    tokio::task::yield_now().await;
    drop(cov_guard);

    timeout(Duration::from_secs(2), async {
        while runtime.health().await.cov_notification_lag_count == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let events = runtime.next_events(32, Duration::ZERO).await;
    let skipped = events
        .events
        .iter()
        .find_map(|event| match event.kind {
            EventKind::CovNotificationLagged { skipped } => Some(skipped),
            _ => None,
        })
        .expect("receiver lag event");
    let delivered = events
        .events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::CovNotification { .. }))
        .count() as u64;
    assert_eq!(skipped + delivered, 5);
    assert_eq!(runtime.health().await.cov_notification_lag_count, skipped);
    runtime
        .apply_observation_plan(ObservationPlan {
            revision: 2,
            subscriptions: Vec::new(),
        })
        .await
        .unwrap();
    server_task.await.unwrap();
    runtime.stop().await.unwrap();
}

