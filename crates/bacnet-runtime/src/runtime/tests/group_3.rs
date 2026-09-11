#[tokio::test]
async fn concurrent_replacement_and_plan_admit_one_cov_pump() {
    let attachment_id = AttachmentId::from(18);
    let current = attachment(18, "concurrent-before");
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![current.clone()],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let old_epoch = runtime
        .inner
        .registry
        .read()
        .await
        .attachment_epoch(attachment_id)
        .unwrap();
    let stable_task_count = runtime.inner.supervisor.task_count();

    // Hold the lifecycle admission gate while reconcile installs the new
    // transport epoch. Reconcile's refresh and the plan's pump preflight
    // then queue concurrently behind the same deterministic barrier.
    let lifecycle = runtime.inner.cov_pump_lifecycle.lock().await;
    let mut replacement = current;
    replacement.label = "concurrent-after".to_owned();
    let TransportConfig::Bip(bip) = &mut replacement.transport else {
        unreachable!()
    };
    bip.broadcast = Some("127.255.255.254".to_owned());
    let reconcile_runtime = runtime.clone();
    let reconcile =
        tokio::spawn(async move { reconcile_runtime.reconcile(2, vec![replacement]).await });
    timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .inner
                .registry
                .read()
                .await
                .attachment_epoch(attachment_id)
                != Some(old_epoch)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("replacement did not install a new epoch");

    let start_barrier = Arc::new(tokio::sync::Barrier::new(2));
    let plan_runtime = runtime.clone();
    let plan_barrier = Arc::clone(&start_barrier);
    let plan = tokio::spawn(async move {
        plan_barrier.wait().await;
        plan_runtime
            .apply_observation_plan(ObservationPlan {
                revision: 1,
                subscriptions: vec![ObservationSpec {
                    key: ObservationKey {
                        device: DeviceKey {
                            attachment_id,
                            device_instance: 400_018,
                        },
                        object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                        object_instance: 17,
                        subscriber_process_id: 180_018,
                    },
                    confirmed: false,
                    lifetime_seconds: 60,
                    renewal_margin: Duration::from_secs(10),
                    suppress_poll_when_fresh: false,
                }],
            })
            .await
    });
    start_barrier.wait().await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    drop(lifecycle);

    assert_eq!(
        reconcile.await.unwrap().unwrap().updated,
        vec![attachment_id]
    );
    assert_eq!(
        plan.await.unwrap().unwrap_err().code,
        ErrorCode::DeviceUnavailable
    );
    timeout(Duration::from_secs(2), async {
        while runtime.inner.supervisor.task_count() != stable_task_count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("retired COV pump did not exit");
    assert_eq!(runtime.inner.cov_pumps.lock().await.len(), 1);

    let _ = runtime.next_events(32, Duration::ZERO).await;
    let destination = runtime
        .inner
        .registry
        .read()
        .await
        .transport(attachment_id)
        .unwrap()
        .local_mac()
        .to_vec();
    let mut sender = NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    sender.start().await.unwrap();
    sender
        .send_apdu(
            &unsolicited_cov_apdu(180_018, 400_018),
            &destination,
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();
    let first = timeout(Duration::from_secs(2), async {
        loop {
            let events = runtime
                .next_events(8, Duration::from_millis(20))
                .await
                .events;
            if events.iter().any(|event| {
                matches!(
                    event.kind,
                    EventKind::UnsolicitedCovNotification { ref notification }
                        if notification.subscriber_process_identifier == 180_018
                )
            }) {
                break events;
            }
        }
    })
    .await
    .expect("concurrently admitted COV pump did not deliver");
    let mut matching = first
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::UnsolicitedCovNotification { ref notification }
                    if notification.subscriber_process_identifier == 180_018
            )
        })
        .count();
    tokio::time::sleep(Duration::from_millis(50)).await;
    matching += runtime
        .next_events(8, Duration::ZERO)
        .await
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::UnsolicitedCovNotification { ref notification }
                    if notification.subscriber_process_identifier == 180_018
            )
        })
        .count();
    assert_eq!(matching, 1);
    assert_eq!(runtime.inner.cov_pumps.lock().await.len(), 1);
    assert_eq!(runtime.inner.supervisor.task_count(), stable_task_count);

    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
    assert!(runtime.inner.cov_pumps.lock().await.is_empty());
    assert_eq!(runtime.inner.supervisor.task_count(), 0);
}

#[tokio::test]
async fn occupied_bip_port_maps_to_bind_conflict_without_leaking_runtime() {
    let blocker = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = blocker.local_addr().unwrap().port();
    let error = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment_on_port(9, "conflict", port)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::BindConflict);
    assert_eq!(error.attachment_id, Some(AttachmentId::from(9)));
    assert!(error.retryable);
}

#[tokio::test]
async fn one_hundred_start_stop_cycles_restore_socket_task_and_registry_baselines() {
    for cycle in 0..100_u128 {
        let reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);

        let runtime = BacnetRuntime::start(RuntimeConfig {
            attachments: vec![attachment_on_port(cycle + 1, "cycle", port)],
            ..RuntimeConfig::default()
        })
        .await
        .unwrap();
        let running = runtime.health().await;
        // Runtime work loop + I-Am/COV pumps + client dispatch + B/IP receive task.
        assert_eq!(running.task_count, 5);
        assert_eq!(running.attachments.len(), 1);

        let report = runtime.stop().await.unwrap();
        assert_eq!(report.tasks_joined, 3);
        assert_eq!(report.tasks_aborted, 0);
        assert_eq!(report.remaining_attachments, 0);
        let stopped = runtime.health().await;
        assert_eq!(stopped.task_count, 0);
        assert!(stopped.attachments.is_empty());
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).unwrap();
    }
}

#[tokio::test]
async fn discovery_merges_duplicate_devices_from_multiple_attachments() {
    let first = AttachmentId::from(1);
    let second = AttachmentId::from(2);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "first"), attachment(2, "second")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    {
        let registry = runtime.inner.registry.read().await;
        registry
            .seed_device(first, 100, &[192, 0, 2, 10, 0xba, 0xc1])
            .await;
        registry.seed_device(second, 100, &[5]).await;
    }

    let snapshot = runtime
        .discover(crate::DiscoveryRequest {
            observation_window: Duration::from_millis(1),
            collect_routers: false,
            ..crate::DiscoveryRequest::default()
        })
        .await
        .unwrap();
    assert!(snapshot.errors.is_empty());
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(
        snapshot.devices[0]
            .selected
            .as_ref()
            .unwrap()
            .key
            .attachment_id,
        first
    );
    assert_eq!(snapshot.devices[0].alternates.len(), 1);
    assert_eq!(snapshot.devices[0].alternates[0].key.attachment_id, second);
    let health = runtime.health().await;
    assert_eq!(health.device_count, 1);
    assert_eq!(health.device_observation_count, 2);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn discovery_reports_unknown_attachment_without_failing_healthy_results() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "healthy")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let missing = AttachmentId::from(99);
    let snapshot = runtime
        .discover(crate::DiscoveryRequest {
            attachment_ids: vec![AttachmentId::from(1), missing],
            observation_window: Duration::from_millis(1),
            collect_routers: false,
            ..crate::DiscoveryRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(snapshot.errors.len(), 1);
    assert_eq!(snapshot.errors[0].code, ErrorCode::AttachmentNotFound);
    assert_eq!(snapshot.errors[0].attachment_id, Some(missing));
    assert!(runtime.health().await.accepting_commands);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn topology_walk_reads_real_bdt_and_fdt_from_seed_bbmd() {
    let reservation = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let bbmd_port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut bbmd = BipTransport::new(
        Ipv4Addr::LOCALHOST,
        bbmd_port,
        Ipv4Addr::new(127, 255, 255, 255),
    );
    bbmd.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: bbmd_port,
        broadcast_mask: [255, 0, 0, 0],
    }]);
    bbmd.enable_foreign_device_registration(
        bacnet_transport::bip::ForeignDevicePolicy::default(),
    );
    let _bbmd_rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port,
        ttl: 4,
    });
    let _foreign_rx = foreign.start().await.unwrap();
    let foreign_mac = foreign.local_mac().to_vec();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let attachment_id = AttachmentId::from(1);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "topology-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let topology = runtime
        .topology(crate::TopologyRequest {
            bbmd_seeds: vec![crate::BbmdTarget {
                attachment_id,
                mac: bbmd_mac.clone(),
            }],
            ..crate::TopologyRequest::default()
        })
        .await
        .unwrap();
    assert!(topology.errors.is_empty());
    assert_eq!(topology.attachments.len(), 1);
    let attachment = &topology.attachments[0];
    assert_eq!(attachment.attachment_id, attachment_id);
    assert_eq!(attachment.bbmds.len(), 1);
    assert_eq!(attachment.bbmds[0].mac, bbmd_mac);
    assert_eq!(
        attachment.bbmds[0].bdt,
        vec![crate::BdtRecord {
            ip: [127, 0, 0, 1],
            port: bbmd_port,
            broadcast_mask: [255, 0, 0, 0],
        }]
    );
    assert_eq!(attachment.bbmds[0].fdt.len(), 1);
    let first_fdt = &attachment.bbmds[0].fdt[0];
    assert_eq!(first_fdt.ip, foreign_mac[..4]);
    assert_eq!(
        first_fdt.port,
        u16::from_be_bytes([foreign_mac[4], foreign_mac[5]])
    );
    assert_eq!(first_fdt.ttl, 4);
    assert!(!attachment.truncated);

    tokio::time::sleep(Duration::from_millis(1100)).await;
    let later = runtime
        .topology(crate::TopologyRequest {
            bbmd_seeds: vec![crate::BbmdTarget {
                attachment_id,
                mac: bbmd_mac,
            }],
            ..crate::TopologyRequest::default()
        })
        .await
        .unwrap();
    let later_fdt = &later.attachments[0].bbmds[0].fdt[0];
    assert!(
        later_fdt.seconds_remaining < first_fdt.seconds_remaining,
        "FDT topology must report the live expiry countdown"
    );

    runtime.stop().await.unwrap();
    foreign.stop().await.unwrap();
    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn topology_rejects_lossy_bip_seed_before_network_io() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "topology-client")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let error = runtime
        .topology(crate::TopologyRequest {
            bbmd_seeds: vec![crate::BbmdTarget {
                attachment_id: AttachmentId::from(1),
                mac: vec![127, 0, 0, 1],
            }],
            ..crate::TopologyRequest::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    assert_eq!(error.attachment_id, Some(AttachmentId::from(1)));
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn scan_rejects_rpm_then_completes_read_property_fallback() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = NetworkLayer::new(transport);
    let mut server_rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let server_task = tokio::spawn(async move {
        let rpm = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let rpm_request = match apdu::decode_apdu(rpm.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected RPM request, got {other:?}"),
        };
        assert_eq!(
            rpm_request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
        );
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::Reject(RejectPdu {
                invoke_id: rpm_request.invoke_id,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            }),
        )
        .unwrap();
        server
            .send_apdu(&encoded, &rpm.source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();

        let rp = timeout(Duration::from_secs(2), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let rp_request = match apdu::decode_apdu(rp.apdu).unwrap() {
            Apdu::ConfirmedRequest(request) => request,
            other => panic!("expected RP request, got {other:?}"),
        };
        assert_eq!(
            rp_request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY
        );
        let read = ReadPropertyRequest::decode(&rp_request.service_request).unwrap();
        let ack = ReadPropertyACK {
            object_identifier: read.object_identifier,
            property_identifier: read.property_identifier,
            property_array_index: read.property_array_index,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: rp_request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from(service_ack.to_vec()),
            }),
        )
        .unwrap();
        server
            .send_apdu(&encoded, &rp.source_mac, false, NetworkPriority::NORMAL)
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
        attachments: vec![attachment(1, "scan-client")],
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
                mac: server_mac.clone(),
            },
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    let snapshot = runtime
        .scan(ScanRequest {
            device,
            reads: vec![PropertyRead {
                input_index: 7,
                object_type: ObjectType::ANALOG_INPUT.to_raw() as u16,
                object_instance: 1,
                property_id: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                array_index: None,
                value_category: "real".to_owned(),
            }],
            limits: PlanLimits {
                max_request_bytes: 1476,
                max_estimated_response_bytes: 1476,
                max_properties: 8,
            },
            progress_interval: Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(snapshot.rpm_attempts, 1);
    assert_eq!(snapshot.rp_fallbacks, 1);
    assert_eq!(snapshot.errors[0].code, ErrorCode::Reject);
    assert_eq!(snapshot.outcomes[0].input_index, 7);
    assert_eq!(
        snapshot.outcomes[0].raw_value,
        Some(vec![0x44, 0x42, 0x90, 0x00, 0x00])
    );
    assert_eq!(snapshot.path, DevicePath::Direct { mac: server_mac });
    let events = runtime.next_events(10, Duration::ZERO).await;
    assert!(events.events.iter().any(|event| {
        event.attachment_id == Some(attachment_id)
            && event.kind
                == EventKind::ScanProgress {
                    device_instance: 100,
                    completed_batches: 1,
                    total_batches: 1,
                    final_update: true,
                }
    }));
    assert_eq!(
        runtime.inner.capabilities.read().await.get(device).rpm,
        Support::Unsupported
    );
    server_task.await.unwrap();
    runtime.stop().await.unwrap();
}
