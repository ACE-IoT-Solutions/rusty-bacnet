#[tokio::test]
async fn ordered_write_batch_preserves_priority_authorization_and_verified_readback() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let raw_value = vec![0x44, 0x42, 0x90, 0x00, 0x00];
    let expected = raw_value.clone();
    let server_task = tokio::spawn(async move {
        let write_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let write_apdu = match apdu::decode_apdu(write_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected WriteProperty, got {other:?}"),
        };
        assert_eq!(
            write_apdu.service_choice,
            ConfirmedServiceChoice::WRITE_PROPERTY
        );
        let write = WritePropertyRequest::decode(&write_apdu.service_request).unwrap();
        assert_eq!(write.priority, Some(8));
        assert_eq!(write.property_value, expected);
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: write_apdu.invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &write_message.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();

        let read_message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let read_apdu = match apdu::decode_apdu(read_message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected readback, got {other:?}"),
        };
        let read = ReadPropertyRequest::decode(&read_apdu.service_request).unwrap();
        let ack = ReadPropertyACK {
            object_identifier: read.object_identifier,
            property_identifier: read.property_identifier,
            property_array_index: read.property_array_index,
            property_value: expected,
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: read_apdu.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from(service_ack.to_vec()),
            }),
        )
        .unwrap();
        server
            .send_apdu(
                &encoded,
                &read_message.source_mac,
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
        attachments: vec![attachment(1, "write-client")],
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
    let target = PropertyRead {
        input_index: 0,
        object_type: ObjectType::ANALOG_OUTPUT.to_raw() as u16,
        object_instance: 1,
        property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        array_index: None,
        value_category: "real".to_owned(),
    };
    let outcomes = runtime
        .write_batch(WriteBatch {
            items: vec![WriteBatchItem {
                input_index: 9,
                device,
                target: target.clone(),
                raw_value: raw_value.clone(),
                bacnet_priority: Some(8),
                authorization_id: "decision-123".to_owned(),
                verify_readback: true,
            }],
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap();
    assert_eq!(outcomes[0].input_index, 9);
    assert_eq!(outcomes[0].authorization_id, "decision-123");
    assert!(outcomes[0].written);
    assert_eq!(outcomes[0].readback, Some(raw_value));
    assert!(outcomes[0].error.is_none());
    server_task.await.unwrap();
    let cached = runtime
        .read_batch(ReadBatch {
            items: vec![ReadBatchItem {
                input_index: 4,
                device,
                read: target,
                freshness: FreshnessPolicy::MaxAge(Duration::from_secs(1)),
            }],
            deadline: std::time::Instant::now() + Duration::from_secs(1),
            priority: WorkPriority::Poll,
        })
        .await
        .unwrap();
    assert_eq!(cached[0].source, Some(ReadSource::Cache));
    assert!(cached[0].error.is_none());
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn unverified_same_device_writes_use_one_wpm_and_invalidate_cache() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let first_value = vec![0x44, 0x42, 0x90, 0x00, 0x00];
    let second_value = vec![0x44, 0x42, 0x92, 0x00, 0x00];
    let expected_first = first_value.clone();
    let expected_second = second_value.clone();
    let server_task = tokio::spawn(async move {
        let message = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let request = match apdu::decode_apdu(message.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected WritePropertyMultiple, got {other:?}"),
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
        );
        let write = WritePropertyMultipleRequest::decode(&request.service_request).unwrap();
        assert_eq!(write.list_of_write_access_specs.len(), 1);
        let spec = &write.list_of_write_access_specs[0];
        assert_eq!(
            spec.object_identifier.object_type(),
            ObjectType::ANALOG_OUTPUT
        );
        assert_eq!(spec.object_identifier.instance_number(), 1);
        assert_eq!(spec.list_of_properties.len(), 2);
        assert_eq!(
            spec.list_of_properties[0].property_identifier,
            PropertyIdentifier::PRESENT_VALUE
        );
        assert_eq!(spec.list_of_properties[0].value, expected_first);
        assert_eq!(spec.list_of_properties[0].priority, Some(8));
        assert_eq!(
            spec.list_of_properties[1].property_identifier,
            PropertyIdentifier::RELINQUISH_DEFAULT
        );
        assert_eq!(spec.list_of_properties[1].value, expected_second);
        assert_eq!(spec.list_of_properties[1].priority, None);

        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
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
        attachments: vec![attachment(1, "wpm-client")],
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
    let first = PropertyRead {
        input_index: 0,
        object_type: ObjectType::ANALOG_OUTPUT.to_raw() as u16,
        object_instance: 1,
        property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        array_index: None,
        value_category: "real".to_owned(),
    };
    let second = PropertyRead {
        property_id: PropertyIdentifier::RELINQUISH_DEFAULT.to_raw(),
        ..first.clone()
    };
    {
        let mut cache = runtime.inner.values.write().await;
        let now = std::time::Instant::now();
        cache.insert(device, &first, vec![0x91, 0], now);
        cache.insert(device, &second, vec![0x91, 0], now);
    }
    let outcomes = runtime
        .write_batch(WriteBatch {
            items: vec![
                WriteBatchItem {
                    input_index: 9,
                    device,
                    target: first.clone(),
                    raw_value: first_value,
                    bacnet_priority: Some(8),
                    authorization_id: "decision-first".to_owned(),
                    verify_readback: false,
                },
                WriteBatchItem {
                    input_index: 2,
                    device,
                    target: second.clone(),
                    raw_value: second_value,
                    bacnet_priority: None,
                    authorization_id: "decision-second".to_owned(),
                    verify_readback: false,
                },
            ],
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| (outcome.input_index, outcome.authorization_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "decision-second"), (9, "decision-first")]
    );
    assert!(outcomes
        .iter()
        .all(|outcome| outcome.written && outcome.readback.is_none() && outcome.error.is_none()));
    server_task.await.unwrap();
    let cache = runtime.inner.values.read().await;
    let now = std::time::Instant::now();
    assert!(cache
        .get_fresh(device, &first, Duration::from_secs(60), now)
        .is_none());
    assert!(cache
        .get_fresh(device, &second, Duration::from_secs(60), now)
        .is_none());
    drop(cache);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn restored_routed_path_reads_without_discovery_and_preserves_dadr() {
    let mut router = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut router_rx = router.start().await.unwrap();
    let router_mac = router.local_mac().to_vec();
    let remote_network = 2001;
    let remote_mac = vec![0x00, 0x2a];
    let expected_remote_mac = remote_mac.clone();
    let raw_value = vec![0x44, 0x41, 0x9c, 0x00, 0x00];
    let expected_value = raw_value.clone();
    let router_task = tokio::spawn(async move {
        let message = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(message.npdu).unwrap();
        let destination = npdu.destination.expect("routed destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(destination.mac_address.as_ref(), expected_remote_mac);
        let request = match apdu::decode_apdu(npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected routed ReadPropertyMultiple, got {other:?}"),
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
        );
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(expected_value),
                    error: None,
                }],
            }],
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        let mut apdu_buf = BytesMut::new();
        encode_apdu(
            &mut apdu_buf,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                service_ack: service_ack.freeze(),
            }),
        )
        .unwrap();
        let response = Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(&expected_remote_mac),
            }),
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        };
        let mut response_buf = BytesMut::new();
        encode_npdu(&mut response_buf, &response).unwrap();
        router
            .send_unicast(&response_buf, message.source_mac.as_ref())
            .await
            .unwrap();
        router.stop().await.unwrap();
    });

    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "restored-routed-read-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    runtime
        .restore_devices(vec![crate::PersistedDevice {
            key: device,
            path: DevicePath::Routed {
                ingress_mac: router_mac.clone(),
                dnet: remote_network,
                dadr: remote_mac.clone(),
            },
            vendor_id: 1,
            max_apdu_length: 1476,
        }])
        .await
        .unwrap();
    let outcomes = runtime
        .read_batch(ReadBatch {
            items: vec![ReadBatchItem {
                input_index: 7,
                device,
                read: PropertyRead {
                    input_index: 0,
                    object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                    object_instance: 1,
                    property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                    array_index: None,
                    value_category: "real".to_owned(),
                },
                freshness: FreshnessPolicy::WireOnly,
            }],
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].input_index, 7);
    assert_eq!(outcomes[0].raw_value, Some(raw_value));
    assert_eq!(
        outcomes[0].path,
        Some(DevicePath::Routed {
            ingress_mac: router_mac,
            dnet: remote_network,
            dadr: remote_mac,
        })
    );
    router_task.await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn routed_write_batch_preserves_dnet_dadr_and_uses_wpm() {
    let mut router = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut router_rx = router.start().await.unwrap();
    let router_mac = router.local_mac().to_vec();
    let remote_network = 2001;
    let remote_mac = vec![0x00, 0x2a];
    let expected_remote_mac = remote_mac.clone();
    let first_value = vec![0x44, 0x42, 0x90, 0x00, 0x00];
    let second_value = vec![0x44, 0x42, 0x92, 0x00, 0x00];
    let expected_first = first_value.clone();
    let expected_second = second_value.clone();
    let router_task = tokio::spawn(async move {
        let message = timeout(Duration::from_secs(2), router_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(message.npdu).unwrap();
        assert!(npdu.expecting_reply);
        let destination = npdu.destination.expect("routed destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(destination.mac_address.as_ref(), expected_remote_mac);
        let request = match apdu::decode_apdu(npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected routed WritePropertyMultiple, got {other:?}"),
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
        );
        let write = WritePropertyMultipleRequest::decode(&request.service_request).unwrap();
        assert_eq!(write.list_of_write_access_specs.len(), 2);
        assert_eq!(
            write.list_of_write_access_specs[0].list_of_properties[0].value,
            expected_first
        );
        assert_eq!(
            write.list_of_write_access_specs[1].list_of_properties[0].value,
            expected_second
        );

        let mut apdu_buf = BytesMut::new();
        encode_apdu(
            &mut apdu_buf,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            }),
        )
        .unwrap();
        let response = Npdu {
            source: Some(NpduAddress {
                network: remote_network,
                mac_address: MacAddr::from_slice(&expected_remote_mac),
            }),
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        };
        let mut response_buf = BytesMut::new();
        encode_npdu(&mut response_buf, &response).unwrap();
        router
            .send_unicast(&response_buf, message.source_mac.as_ref())
            .await
            .unwrap();
        router.stop().await.unwrap();
    });

    let attachment_id = AttachmentId::from(1);
    let device = DeviceKey {
        attachment_id,
        device_instance: 100,
    };
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "routed-wpm-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let restored = runtime
        .restore_devices(vec![crate::PersistedDevice {
            key: device,
            path: DevicePath::Routed {
                ingress_mac: router_mac.clone(),
                dnet: remote_network,
                dadr: remote_mac.clone(),
            },
            vendor_id: 1,
            max_apdu_length: 1476,
        }])
        .await
        .unwrap();
    assert_eq!(restored.added, 1);
    assert_eq!(restored.index_revision, 1);
    let target = |input_index, object_instance| PropertyRead {
        input_index,
        object_type: ObjectType::ANALOG_OUTPUT.to_raw() as u16,
        object_instance,
        property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        array_index: None,
        value_category: "real".to_owned(),
    };
    let outcomes = runtime
        .write_batch(WriteBatch {
            items: vec![
                WriteBatchItem {
                    input_index: 8,
                    device,
                    target: target(0, 1),
                    raw_value: first_value,
                    bacnet_priority: Some(8),
                    authorization_id: "route-first".to_owned(),
                    verify_readback: false,
                },
                WriteBatchItem {
                    input_index: 3,
                    device,
                    target: target(1, 2),
                    raw_value: second_value,
                    bacnet_priority: Some(12),
                    authorization_id: "route-second".to_owned(),
                    verify_readback: false,
                },
            ],
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            priority: WorkPriority::Foreground,
        })
        .await
        .unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| (outcome.input_index, outcome.authorization_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(3, "route-second"), (8, "route-first")]
    );
    assert!(outcomes.iter().all(|outcome| {
        outcome.written
            && outcome.error.is_none()
            && outcome.path
                == Some(DevicePath::Routed {
                    ingress_mac: router_mac.clone(),
                    dnet: remote_network,
                    dadr: remote_mac.clone(),
                })
    }));
    router_task.await.unwrap();
    runtime.stop().await.unwrap();
}

