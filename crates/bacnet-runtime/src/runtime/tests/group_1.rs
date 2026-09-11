#[test]
fn object_list_decoder_preserves_order_proprietary_types_and_leading_values() {
    let mut raw = BytesMut::new();
    let first = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let proprietary = ObjectIdentifier::new(ObjectType::from_raw(128), 0).unwrap();
    encode_app_object_id(&mut raw, &first);
    encode_app_object_id(&mut raw, &proprietary);
    assert_eq!(
        super::decode_object_list(&raw, 2).unwrap(),
        vec![(0, 1), (128, 0)]
    );
    assert_eq!(
        super::decode_object_list(&raw, 1).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
}

fn attachment(id: u128, label: &str) -> AttachmentConfig {
    attachment_on_port(id, label, 0)
}

fn attachment_on_port(id: u128, label: &str, port: u16) -> AttachmentConfig {
    AttachmentConfig {
        id: AttachmentId::from(id),
        label: label.to_owned(),
        transport: TransportConfig::Bip(BipConfig {
            interface: "127.0.0.1".to_owned(),
            port,
            broadcast: Some("127.255.255.255".to_owned()),
            bbmd_address: None,
            foreign_device_ttl: None,
        }),
    }
}

fn foreign_attachment(
    id: u128,
    label: &str,
    bbmd_address: SocketAddrV4,
    ttl: u16,
) -> AttachmentConfig {
    let mut config = attachment(id, label);
    let TransportConfig::Bip(bip) = &mut config.transport else {
        unreachable!()
    };
    bip.bbmd_address = Some(bbmd_address.to_string());
    bip.foreign_device_ttl = Some(ttl);
    config
}

fn i_am_apdu(instance: u32, vendor_id: u16) -> Bytes {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id,
    }
    .encode(&mut service_request);
    let mut encoded = BytesMut::new();
    encode_apdu(
        &mut encoded,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    encoded.freeze()
}

fn unsolicited_cov_apdu(process_id: u32, device_instance: u32) -> Bytes {
    let mut service_request = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: process_id,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, device_instance)
            .unwrap(),
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17).unwrap(),
        time_remaining: 47,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x41, 0xfc, 0x00, 0x00],
            priority: Some(8),
        }],
    }
    .encode(&mut service_request);
    let mut encoded = BytesMut::new();
    encode_apdu(
        &mut encoded,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    encoded.freeze()
}

#[tokio::test]
#[ignore = "upstream 0.11 does not reserve COV receivers during client startup"]
async fn reserved_cov_receiver_retains_notification_dispatched_during_transport_start() {
    let attachment_id = AttachmentId::from(501);
    crate::transport::install_bip_start_cov_hook(
        attachment_id,
        unsolicited_cov_apdu(50_001, 400_501).to_vec(),
    );
    let mut registry =
        crate::registry::AttachmentRegistry::start(&[attachment(501, "staging-window")], 8)
            .await
            .unwrap();
    assert!(registry.has_reserved_cov_receiver(attachment_id));
    let expected_epoch = registry.attachment_epoch(attachment_id).unwrap();

    let (receiver_epoch, mut receiver) = registry.cov_receiver(attachment_id).unwrap();
    assert_eq!(receiver_epoch, expected_epoch);
    assert!(!registry.has_reserved_cov_receiver(attachment_id));
    let received = timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("reserved COV receiver lost a staging-window notification")
        .unwrap();
    assert_eq!(received.notification.subscriber_process_identifier, 50_001);
    assert_eq!(
        received
            .notification
            .initiating_device_identifier
            .instance_number(),
        400_501
    );

    registry.stop_all().await.unwrap();
}

#[tokio::test]
async fn start_preserves_attachment_order_and_emits_started() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(2, "second"), attachment(1, "first")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let health = runtime.health().await;
    assert_eq!(health.generation, 1);
    assert_eq!(health.attachments[0].label, "second");
    assert_eq!(health.attachments[1].label, "first");
    let batch = runtime.next_events(10, Duration::ZERO).await;
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].kind, EventKind::RuntimeStarted);
}

#[tokio::test]
async fn mixed_foreign_and_normal_health_is_ordered_and_reconciles_independently() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    bbmd.enable_bbmd(Vec::new());
    let _bbmd_rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac();
    let bbmd_address = SocketAddrV4::new(
        Ipv4Addr::new(bbmd_mac[0], bbmd_mac[1], bbmd_mac[2], bbmd_mac[3]),
        u16::from_be_bytes([bbmd_mac[4], bbmd_mac[5]]),
    );

    let foreign = foreign_attachment(2, "foreign", bbmd_address, 120);
    let normal = attachment(1, "normal");
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![foreign.clone(), normal.clone()],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let initial = runtime.health().await;
    assert_eq!(
        initial
            .attachments
            .iter()
            .map(|health| health.id)
            .collect::<Vec<_>>(),
        vec![AttachmentId::from(2), AttachmentId::from(1)]
    );
    // Upstream 0.11 configures and renews foreign registration but does not
    // expose registration telemetry through the client boundary.
    assert!(initial.attachments[0].foreign_device_registration.is_none());
    assert!(initial.attachments[1].foreign_device_registration.is_none());

    let reordered = runtime
        .reconcile(2, vec![normal.clone(), foreign])
        .await
        .unwrap();
    assert!(reordered.added.is_empty());
    assert!(reordered.updated.is_empty());
    assert!(reordered.removed.is_empty());
    let reordered_health = runtime.health().await;
    assert_eq!(
        reordered_health
            .attachments
            .iter()
            .map(|health| health.id)
            .collect::<Vec<_>>(),
        vec![AttachmentId::from(1), AttachmentId::from(2)]
    );
    assert!(reordered_health.attachments[0]
        .foreign_device_registration
        .is_none());
    assert!(reordered_health.attachments[1]
        .foreign_device_registration
        .is_none());

    let removed = runtime.reconcile(3, vec![normal]).await.unwrap();
    assert_eq!(removed.removed, vec![AttachmentId::from(2)]);
    let final_health = runtime.health().await;
    assert_eq!(final_health.attachments.len(), 1);
    assert!(final_health.attachments[0]
        .foreign_device_registration
        .is_none());

    runtime.stop().await.unwrap();
    bbmd.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "upstream 0.11 does not expose live foreign-device registration telemetry"]
async fn background_foreign_registration_health_tracks_live_transitions_in_order() {
    // Scripted BBMD response modes: 0 drops, 1 succeeds, 2 rejects.
    let socket = Arc::new(
        tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let bbmd_address = match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        std::net::SocketAddr::V6(_) => unreachable!(),
    };
    let (mode, mut mode_rx) = tokio::sync::watch::channel(0_u8);

    let foreign = foreign_attachment(2, "foreign", bbmd_address, 1);
    let normal = attachment(1, "normal");
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![foreign, normal],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let pending = runtime.health().await;
    assert_eq!(
        pending
            .attachments
            .iter()
            .map(|health| health.id)
            .collect::<Vec<_>>(),
        vec![AttachmentId::from(2), AttachmentId::from(1)]
    );
    assert_eq!(
        pending.attachments[0].state,
        crate::AttachmentState::Running
    );
    assert_eq!(pending.attachments[0].last_error, None);
    assert_eq!(
        pending.attachments[0]
            .foreign_device_registration
            .as_ref()
            .unwrap()
            .state,
        crate::ForeignDeviceRegistrationState::Pending
    );
    assert!(pending.attachments[1].foreign_device_registration.is_none());

    let responder_socket = Arc::clone(&socket);
    let responder = tokio::spawn(async move {
        let mut buffer = [0_u8; 64];
        let mut last_sender = None;
        loop {
            let response_mode = tokio::select! {
                received = responder_socket.recv_from(&mut buffer) => {
                    let (_, sender) = received.unwrap();
                    last_sender = Some(sender);
                    *mode_rx.borrow()
                }
                changed = mode_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    *mode_rx.borrow_and_update()
                }
            };
            let Some(sender) = last_sender else {
                continue;
            };
            let result_code = match response_mode {
                1 => 0x0000_u16,
                2 => 0x0030_u16,
                _ => continue,
            };
            let code = result_code.to_be_bytes();
            responder_socket
                .send_to(&[0x81, 0x00, 0x00, 0x06, code[0], code[1]], sender)
                .await
                .unwrap();
        }
    });

    async fn wait_for_state(
        runtime: &BacnetRuntime,
        expected: crate::ForeignDeviceRegistrationState,
    ) -> crate::AttachmentHealth {
        timeout(Duration::from_secs(5), async {
            loop {
                let health = runtime.health().await.attachments.remove(0);
                if health
                    .foreign_device_registration
                    .as_ref()
                    .is_some_and(|status| status.state == expected)
                {
                    return health;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }

    mode.send(1).unwrap();
    let registered =
        wait_for_state(&runtime, crate::ForeignDeviceRegistrationState::Registered).await;
    assert_eq!(registered.state, crate::AttachmentState::Running);
    assert_eq!(registered.last_error, None);

    mode.send(2).unwrap();
    let rejected = wait_for_state(&runtime, crate::ForeignDeviceRegistrationState::Rejected).await;
    assert_eq!(rejected.state, crate::AttachmentState::Running);
    assert_eq!(
        rejected.last_error,
        Some(ErrorCode::ForeignDeviceRegistrationFailed)
    );

    mode.send(1).unwrap();
    let recovered =
        wait_for_state(&runtime, crate::ForeignDeviceRegistrationState::Registered).await;
    assert_eq!(recovered.state, crate::AttachmentState::Running);
    assert_eq!(recovered.last_error, None);

    mode.send(0).unwrap();
    let expired = wait_for_state(&runtime, crate::ForeignDeviceRegistrationState::Expired).await;
    assert_eq!(expired.state, crate::AttachmentState::Running);
    assert_eq!(
        expired.last_error,
        Some(ErrorCode::ForeignDeviceRegistrationFailed)
    );

    mode.send(1).unwrap();
    let recovered =
        wait_for_state(&runtime, crate::ForeignDeviceRegistrationState::Registered).await;
    assert_eq!(recovered.state, crate::AttachmentState::Running);
    assert_eq!(recovered.last_error, None);

    runtime.stop().await.unwrap();
    responder.abort();
}

#[tokio::test]
async fn raw_i_am_observations_preserve_duplicates_and_attachment_provenance() {
    let attachment_id = AttachmentId::from(41);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(41, "i-am-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let destination = runtime
        .inner
        .registry
        .read()
        .await
        .transport(attachment_id)
        .unwrap()
        .local_mac()
        .to_vec();
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut sender = NetworkLayer::new(transport);
    sender.start().await.unwrap();
    let sender_mac = sender.local_mac().to_vec();
    let announcement = i_am_apdu(400_041, 808);
    for _ in 0..2 {
        sender
            .send_apdu(&announcement, &destination, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
    }

    let observations = timeout(Duration::from_secs(2), async {
        let mut observations = Vec::new();
        while observations.len() < 2 {
            observations.extend(
                runtime
                    .next_events(8, Duration::from_millis(20))
                    .await
                    .events
                    .into_iter()
                    .filter(|event| matches!(event.kind, EventKind::IAmObservation { .. })),
            );
        }
        observations
    })
    .await
    .unwrap();
    assert!(observations
        .iter()
        .all(|event| event.attachment_id == Some(attachment_id)));
    for event in observations {
        let EventKind::IAmObservation { observation } = event.kind else {
            unreachable!();
        };
        assert_eq!(observation.device_instance, 400_041);
        assert_eq!(observation.vendor_id, 808);
        assert_eq!(observation.source_mac.as_slice(), sender_mac.as_slice());
        assert_eq!(observation.udp_source_ip, Some([127, 0, 0, 1]));
        assert_eq!(
            observation.udp_source_port,
            Some(u16::from_be_bytes([sender_mac[4], sender_mac[5]]))
        );
        assert_eq!(
            observation.bvlc_function,
            Some(BvlcFunction::ORIGINAL_UNICAST_NPDU.to_raw())
        );
        assert_eq!(observation.source_network, None);
        assert_eq!(observation.source_address, None);
        assert_eq!(observation.forwarded_from_ip, None);
        assert_eq!(observation.forwarded_from_port, None);
    }
    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn real_i_am_receiver_overflow_is_visible_in_events_and_health() {
    let attachment_id = AttachmentId::from(51);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(51, "i-am-lag-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let destination = runtime
        .inner
        .registry
        .read()
        .await
        .transport(attachment_id)
        .unwrap()
        .local_mac()
        .to_vec();
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut sender = NetworkLayer::new(transport);
    sender.start().await.unwrap();

    // Hold the registry writer while the first observation reaches the
    // pump. The pump must acquire a registry reader before publishing,
    // so the bounded client receiver deterministically overflows while
    // the remaining announcements are dispatched.
    let registry_guard = runtime.inner.registry.write().await;
    let announcement = i_am_apdu(400_051, 909);
    for _ in 0..96 {
        sender
            .send_apdu(&announcement, &destination, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(registry_guard);

    let skipped = timeout(Duration::from_secs(2), async {
        loop {
            for event in runtime
                .next_events(32, Duration::from_millis(20))
                .await
                .events
            {
                if event.attachment_id == Some(attachment_id) {
                    if let EventKind::IAmObservationLagged { skipped } = event.kind {
                        return skipped;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(skipped > 0);
    assert_eq!(runtime.health().await.i_am_observation_lag_count, skipped);

    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_config_is_rejected_before_runtime_creation() {
    let duplicate = attachment(1, "duplicate");
    let error = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![duplicate.clone(), duplicate],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    assert!(!error.retryable);
}

#[tokio::test]
async fn stop_is_idempotent_and_clears_attachment_ownership() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "primary")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    runtime
        .restore_devices(vec![crate::PersistedDevice {
            key: DeviceKey {
                attachment_id: AttachmentId::from(1),
                device_instance: 100,
            },
            path: DevicePath::Direct {
                mac: vec![127, 0, 0, 1, 0xba, 0xc0],
            },
            vendor_id: 42,
            max_apdu_length: 1476,
        }])
        .await
        .unwrap();

    let first = runtime.stop().await.unwrap();
    assert!(first.newly_stopped);
    assert_eq!(first.remaining_attachments, 0);
    assert!(!runtime.health().await.accepting_commands);
    assert!(runtime.health().await.attachments.is_empty());
    let stopped_health = runtime.health().await;
    assert_eq!(stopped_health.device_count, 0);
    assert_eq!(stopped_health.device_observation_count, 0);
    assert_eq!(stopped_health.capability_count, 0);
    assert_eq!(stopped_health.cached_value_count, 0);
    assert_eq!(stopped_health.observation_count, 0);
    assert_eq!(stopped_health.event_lag_count, 0);
    assert_eq!(stopped_health.cov_notification_lag_count, 0);
    assert_eq!(stopped_health.event_queue_depth, 1);
    let events = runtime.next_events(10, Duration::ZERO).await;
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.events[0].kind, EventKind::RuntimeStopped);

    let second = runtime.stop().await.unwrap();
    assert!(!second.newly_stopped);
    assert_eq!(second.remaining_attachments, 0);
}
