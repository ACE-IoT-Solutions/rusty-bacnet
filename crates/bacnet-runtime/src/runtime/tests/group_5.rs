#[tokio::test]
async fn batch_validation_and_missing_devices_return_stable_ordered_results() {
    let attachment_id = AttachmentId::from(1);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "batch-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let property = PropertyRead {
        input_index: 0,
        object_type: 0,
        object_instance: 1,
        property_id: 85,
        array_index: None,
        value_category: "real".to_owned(),
    };
    let key = |instance| DeviceKey {
        attachment_id,
        device_instance: instance,
    };
    let outcomes = runtime
        .read_batch(ReadBatch {
            items: vec![
                ReadBatchItem {
                    input_index: 9,
                    device: key(9),
                    read: property.clone(),
                    freshness: FreshnessPolicy::WireOnly,
                },
                ReadBatchItem {
                    input_index: 2,
                    device: key(2),
                    read: property.clone(),
                    freshness: FreshnessPolicy::WireOnly,
                },
            ],
            deadline: std::time::Instant::now() + Duration::from_secs(1),
            priority: WorkPriority::Poll,
        })
        .await
        .unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.input_index)
            .collect::<Vec<_>>(),
        vec![2, 9]
    );
    assert!(outcomes.iter().all(|outcome| {
        outcome.error.as_ref().unwrap().code == ErrorCode::DeviceUnavailable
            && outcome.path.is_none()
            && outcome.source.is_none()
    }));

    let error = runtime
        .write_batch(WriteBatch {
            items: vec![WriteBatchItem {
                input_index: 0,
                device: key(1),
                target: property,
                raw_value: vec![0],
                bacnet_priority: Some(17),
                authorization_id: "authorized".to_owned(),
                verify_readback: false,
            }],
            deadline: std::time::Instant::now() + Duration::from_secs(1),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn runtime_cancels_queued_and_inflight_batches_without_leaking_tasks() {
    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "cancel-client")],
        max_inflight_operations: 1,
        scheduler_capacity: 2,
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
            path: DevicePath::Direct {
                mac: vec![127, 0, 0, 1, 0, 9],
            },
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    let batch = |index| ReadBatch {
        items: vec![ReadBatchItem {
            input_index: index,
            device,
            read: PropertyRead {
                input_index: index,
                object_type: 0,
                object_instance: 1,
                property_id: 85,
                array_index: None,
                value_category: "real".to_owned(),
            },
            freshness: FreshnessPolicy::WireOnly,
        }],
        deadline: std::time::Instant::now() + Duration::from_secs(10),
        priority: WorkPriority::Background,
    };
    let first = runtime.submit_read_batch(batch(1)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.inner.inflight.lock().await.get(&first.id).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = runtime.submit_read_batch(batch(2)).await.unwrap();
    let queued = runtime.cancel(second.id).await;
    assert!(queued.queued);
    assert!(!queued.in_flight);
    assert_eq!(
        second.result().await.unwrap_err().code,
        ErrorCode::Cancelled
    );

    let inflight = runtime.cancel(first.id).await;
    assert!(!inflight.queued);
    assert!(inflight.in_flight);
    assert_eq!(first.result().await.unwrap_err().code, ErrorCode::Cancelled);
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.health().await.task_count != 5 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn foreground_batch_completes_while_background_wire_read_is_blocked() {
    let attachment_id = AttachmentId::from(1);
    let blocked_device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "latency-client")],
        max_inflight_operations: 2,
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
            key: blocked_device,
            path: DevicePath::Direct {
                mac: vec![127, 0, 0, 1, 0, 9],
            },
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    let item = |device, index| ReadBatchItem {
        input_index: index,
        device,
        read: PropertyRead {
            input_index: index,
            object_type: 0,
            object_instance: 1,
            property_id: 85,
            array_index: None,
            value_category: "real".to_owned(),
        },
        freshness: FreshnessPolicy::WireOnly,
    };
    let background = runtime
        .submit_read_batch(ReadBatch {
            items: vec![item(blocked_device, 0)],
            deadline: std::time::Instant::now() + Duration::from_secs(10),
            priority: WorkPriority::Background,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime
            .inner
            .inflight
            .lock()
            .await
            .get(&background.id)
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let started = std::time::Instant::now();
    let foreground = runtime
        .read_batch(ReadBatch {
            items: vec![item(
                DeviceKey {
                    attachment_id,
                    device_instance: 999,
                },
                1,
            )],
            deadline: std::time::Instant::now() + Duration::from_secs(1),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(
        foreground[0].error.as_ref().unwrap().code,
        ErrorCode::DeviceUnavailable
    );
    runtime.cancel(background.id).await;
    assert_eq!(
        background.result().await.unwrap_err().code,
        ErrorCode::Cancelled
    );
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn observation_plan_subscribes_idempotently_and_unsubscribes_by_revision() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let server_task = tokio::spawn(async move {
        for (subscription_index, expected_lifetime) in
            [Some(2), Some(2), Some(2), None].into_iter().enumerate()
        {
            let message = timeout(Duration::from_secs(2), server_rx.recv())
                .await
                .unwrap()
                .unwrap();
            let request = match apdu::decode_apdu(message.apdu).unwrap() {
                Apdu::ConfirmedRequest(request) => request,
                other => panic!("expected SubscribeCOV, got {other:?}"),
            };
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::SUBSCRIBE_COV
            );
            let subscribe = SubscribeCOVRequest::decode(&request.service_request).unwrap();
            assert_eq!(subscribe.subscriber_process_identifier, 77);
            assert_eq!(subscribe.lifetime, expected_lifetime);
            assert_eq!(
                subscribe.issue_confirmed_notifications,
                expected_lifetime.map(|_| true)
            );
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
            if expected_lifetime.is_some() {
                let notification = COVNotificationRequest {
                    subscriber_process_identifier: 77,
                    initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100)
                        .unwrap(),
                    monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                        .unwrap(),
                    time_remaining: 1,
                    list_of_values: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                        priority: None,
                    }],
                };
                let mut service = BytesMut::new();
                notification.encode(&mut service);
                let mut encoded = BytesMut::new();
                if subscription_index == 1 {
                    encode_apdu(
                        &mut encoded,
                        &Apdu::ConfirmedRequest(apdu::ConfirmedRequest {
                            segmented: false,
                            more_follows: false,
                            segmented_response_accepted: false,
                            max_segments: None,
                            max_apdu_length: 1476,
                            invoke_id: 91,
                            sequence_number: None,
                            proposed_window_size: None,
                            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                            service_request: Bytes::from(service.to_vec()),
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
                    let ack_message = timeout(Duration::from_secs(2), server_rx.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    let ack = apdu::decode_apdu(ack_message.apdu).unwrap();
                    assert_eq!(
                        ack,
                        Apdu::SimpleAck(SimpleAck {
                            invoke_id: 91,
                            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                        })
                    );
                } else {
                    encode_apdu(
                        &mut encoded,
                        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                            service_request: Bytes::from(service.to_vec()),
                        }),
                    )
                    .unwrap();
                    for _ in 0..2 {
                        server
                            .send_apdu(
                                &encoded,
                                &message.source_mac,
                                false,
                                NetworkPriority::NORMAL,
                            )
                            .await
                            .unwrap();
                    }
                }
            }
        }
        server.stop().await.unwrap();
    });

    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "cov-client")],
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
    let spec = ObservationSpec {
        key: ObservationKey {
            device,
            object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
            object_instance: 1,
            subscriber_process_id: 77,
        },
        confirmed: true,
        lifetime_seconds: 2,
        renewal_margin: Duration::from_secs(1),
        suppress_poll_when_fresh: true,
    };
    let added = runtime
        .apply_observation_plan(ObservationPlan {
            revision: 1,
            subscriptions: vec![spec.clone()],
        })
        .await
        .unwrap();
    assert_eq!(added.added, vec![spec.key]);
    assert_eq!(runtime.health().await.observation_count, 1);
    let replay = runtime
        .apply_observation_plan(ObservationPlan {
            revision: 1,
            subscriptions: vec![spec],
        })
        .await
        .unwrap();
    assert!(replay.idempotent);
    let events = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let events = runtime.next_events(16, Duration::from_millis(20)).await;
            if events
                .events
                .iter()
                .any(|event| matches!(event.kind, EventKind::CovNotification { .. }))
            {
                break events;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::CovNotification { .. }))
            .count(),
        1
    );
    let cached = runtime
        .read_batch(ReadBatch {
            items: vec![ReadBatchItem {
                input_index: 0,
                device,
                read: PropertyRead {
                    input_index: 0,
                    object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                    object_instance: 1,
                    property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                    array_index: None,
                    value_category: "real".to_owned(),
                },
                freshness: FreshnessPolicy::MaxAge(Duration::from_secs(1)),
            }],
            deadline: std::time::Instant::now() + Duration::from_secs(1),
            priority: WorkPriority::Poll,
        })
        .await
        .unwrap();
    assert_eq!(cached[0].source, Some(ReadSource::Cache));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let events = runtime.next_events(16, Duration::from_millis(20)).await;
            if events
                .events
                .iter()
                .any(|event| matches!(event.kind, EventKind::CovRenewed { .. }))
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let replacement_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    runtime
        .reconcile(
            2,
            vec![attachment_on_port(
                1,
                "cov-client-reconnected",
                replacement_port,
            )],
        )
        .await
        .unwrap();
    assert_eq!(runtime.health().await.observation_count, 1);
    let removed = runtime
        .apply_observation_plan(ObservationPlan {
            revision: 2,
            subscriptions: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(removed.removed.len(), 1);
    assert_eq!(runtime.health().await.observation_count, 0);
    server_task.await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.health().await.task_count != 5 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.stop().await.unwrap();
}

