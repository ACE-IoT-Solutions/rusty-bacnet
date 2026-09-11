#[tokio::test]
async fn reconcile_add_update_remove_and_reorder_is_revisioned() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "one"), attachment(2, "two")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let report = runtime
        .reconcile(
            2,
            vec![attachment(2, "two-renamed"), attachment(3, "three")],
        )
        .await
        .unwrap();
    assert_eq!(report.added, vec![AttachmentId::from(3)]);
    assert_eq!(report.updated, vec![AttachmentId::from(2)]);
    assert_eq!(report.removed, vec![AttachmentId::from(1)]);
    assert_eq!(report.generation, 2);
    assert!(!report.idempotent);
    assert_eq!(
        runtime
            .health()
            .await
            .attachments
            .iter()
            .map(|health| health.label.as_str())
            .collect::<Vec<_>>(),
        vec!["two-renamed", "three"]
    );
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn repeated_revision_is_idempotent_only_for_identical_configuration() {
    let desired = vec![attachment(1, "one")];
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: desired.clone(),
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();

    let replay = runtime.reconcile(1, desired).await.unwrap();
    assert!(replay.idempotent);
    assert_eq!(replay.generation, 1);

    let error = runtime
        .reconcile(1, vec![attachment(1, "changed")])
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRevision);
    assert_eq!(runtime.health().await.generation, 1);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn persisted_device_restore_is_transactional_idempotent_and_lossless() {
    let attachment_id = AttachmentId::from(1);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "persisted")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let direct = crate::PersistedDevice {
        key: DeviceKey {
            attachment_id,
            device_instance: 100,
        },
        path: DevicePath::Direct {
            mac: vec![127, 0, 0, 1, 0xba, 0xc0],
        },
        vendor_id: 42,
        max_apdu_length: 1476,
    };
    let routed = crate::PersistedDevice {
        key: DeviceKey {
            attachment_id,
            device_instance: 101,
        },
        path: DevicePath::Routed {
            ingress_mac: vec![127, 0, 0, 2, 0xba, 0xc1],
            dnet: 2200,
            dadr: vec![0, 5],
        },
        vendor_id: 43,
        max_apdu_length: 1024,
    };

    let invalid = crate::PersistedDevice {
        key: DeviceKey {
            attachment_id,
            device_instance: 102,
        },
        path: DevicePath::Direct { mac: vec![1] },
        vendor_id: 0,
        max_apdu_length: 1476,
    };
    let error = runtime
        .restore_devices(vec![direct.clone(), invalid])
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    assert_eq!(runtime.health().await.device_observation_count, 0);

    let first = runtime
        .restore_devices(vec![direct.clone(), routed.clone()])
        .await
        .unwrap();
    assert_eq!((first.added, first.updated, first.unchanged), (2, 0, 0));
    assert_eq!(first.index_revision, 2);
    let index = runtime.inner.device_index.read().await;
    assert_eq!(index.get(routed.key).unwrap().path, routed.path);
    drop(index);

    let replay = runtime.restore_devices(vec![direct, routed]).await.unwrap();
    assert_eq!((replay.added, replay.updated, replay.unchanged), (0, 0, 2));
    assert_eq!(replay.index_revision, 2);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn persisted_restore_is_serialized_with_stop_in_both_lock_orders() {
    fn persisted() -> crate::PersistedDevice {
        crate::PersistedDevice {
            key: DeviceKey {
                attachment_id: AttachmentId::from(1),
                device_instance: 100,
            },
            path: DevicePath::Direct {
                mac: vec![127, 0, 0, 1, 0xba, 0xc0],
            },
            vendor_id: 42,
            max_apdu_length: 1476,
        }
    }

    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "persisted")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let gate = runtime.inner.reconcile_lock.lock().await;
    let restore_runtime = runtime.clone();
    let restore =
        tokio::spawn(async move { restore_runtime.restore_devices(vec![persisted()]).await });
    tokio::task::yield_now().await;
    let stop_runtime = runtime.clone();
    let stop = tokio::spawn(async move { stop_runtime.stop().await });
    tokio::task::yield_now().await;
    drop(gate);
    restore.await.unwrap().unwrap();
    stop.await.unwrap().unwrap();
    assert_eq!(runtime.health().await.device_observation_count, 0);

    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "persisted")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let gate = runtime.inner.reconcile_lock.lock().await;
    let stop_runtime = runtime.clone();
    let stop = tokio::spawn(async move { stop_runtime.stop().await });
    tokio::task::yield_now().await;
    let restore_runtime = runtime.clone();
    let restore =
        tokio::spawn(async move { restore_runtime.restore_devices(vec![persisted()]).await });
    tokio::task::yield_now().await;
    drop(gate);
    stop.await.unwrap().unwrap();
    assert_eq!(
        restore.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(runtime.health().await.device_observation_count, 0);
}

#[tokio::test]
async fn failed_addition_isolated_from_running_attachment_and_revision() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, "healthy")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let unsupported = AttachmentConfig {
        id: AttachmentId::from(2),
        label: "mstp-not-enabled".to_owned(),
        transport: TransportConfig::Mstp(MstpConfig {
            device: "/dev/ttyUSB0".to_owned(),
            baud: 38_400,
            mac: 1,
            max_master: 127,
            max_info_frames: 1,
        }),
    };

    let error = runtime
        .reconcile(2, vec![attachment(1, "healthy"), unsupported])
        .await
        .unwrap_err();
    #[cfg(feature = "mstp")]
    assert_eq!(error.code, ErrorCode::SerialUnavailable);
    #[cfg(not(feature = "mstp"))]
    assert_eq!(error.code, ErrorCode::UnsupportedTransport);
    let health = runtime.health().await;
    assert_eq!(health.generation, 1);
    assert!(health.accepting_commands);
    assert_eq!(health.attachments.len(), 1);
    assert_eq!(health.attachments[0].label, "healthy");
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn failed_same_bind_replacement_restores_prior_attachment() {
    let current = attachment(1, "healthy");
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![current.clone()],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let mut invalid = current;
    invalid.label = "invalid replacement".to_owned();
    let TransportConfig::Bip(bip) = &mut invalid.transport else {
        unreachable!()
    };
    bip.broadcast = Some("not-an-ip".to_owned());

    let error = runtime.reconcile(2, vec![invalid]).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    let health = runtime.health().await;
    assert_eq!(health.generation, 1);
    assert_eq!(health.attachments.len(), 1);
    assert_eq!(health.attachments[0].label, "healthy");
    assert_eq!(health.attachments[0].state, crate::AttachmentState::Running);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn failed_runtime_replacement_rebinds_observation_pumps_to_rollback_epoch() {
    let attachment_id = AttachmentId::from(7);
    let current = attachment(7, "rollback-source");
    let other = attachment(8, "staging-failure-source");
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![current.clone(), other.clone()],
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
    assert_eq!(
        runtime.inner.i_am_pumps.lock().await[&attachment_id].attachment_epoch,
        old_epoch
    );
    assert_eq!(
        runtime.inner.cov_pumps.lock().await[&attachment_id].attachment_epoch,
        old_epoch
    );

    let mut replacement = current;
    let TransportConfig::Bip(bip) = &mut replacement.transport else {
        unreachable!()
    };
    // Same-bind replacement succeeds first and therefore requires rollback
    // when staging the following attachment fails.
    bip.broadcast = Some("127.255.255.254".to_owned());

    let occupied = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let occupied_port = occupied.local_addr().unwrap().port();
    let mut failing = other;
    let TransportConfig::Bip(bip) = &mut failing.transport else {
        unreachable!()
    };
    bip.port = occupied_port;

    let error = runtime
        .reconcile(2, vec![replacement, failing])
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::BindConflict);
    let restored_epoch = runtime
        .inner
        .registry
        .read()
        .await
        .attachment_epoch(attachment_id)
        .unwrap();
    assert_ne!(restored_epoch, old_epoch);
    assert_eq!(
        runtime.inner.i_am_pumps.lock().await[&attachment_id].attachment_epoch,
        restored_epoch
    );
    assert_eq!(
        runtime.inner.cov_pumps.lock().await[&attachment_id].attachment_epoch,
        restored_epoch
    );
    assert!(!runtime
        .inner
        .registry
        .read()
        .await
        .has_reserved_i_am_receiver(attachment_id));
    assert!(!runtime
        .inner
        .registry
        .read()
        .await
        .has_reserved_cov_receiver(attachment_id));

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
            &i_am_apdu(400_007, 707),
            &destination,
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();
    let recovered = timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .next_events(8, Duration::from_millis(20))
                .await
                .events
                .into_iter()
                .any(|event| matches!(event.kind, EventKind::IAmObservation { .. }))
            {
                break;
            }
        }
    })
    .await;
    assert!(
        recovered.is_ok(),
        "rollback-restored I-Am pump was stranded"
    );

    sender
        .send_apdu(
            &unsolicited_cov_apdu(70_007, 400_007),
            &destination,
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();
    let recovered = timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .next_events(8, Duration::from_millis(20))
                .await
                .events
                .into_iter()
                .any(|event| {
                    matches!(
                        event.kind,
                        EventKind::UnsolicitedCovNotification { ref notification }
                            if notification.subscriber_process_identifier == 70_007
                    )
                })
            {
                break;
            }
        }
    })
    .await;
    assert!(recovered.is_ok(), "rollback-restored COV pump was stranded");

    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
    drop(occupied);
}

#[tokio::test]
async fn label_only_reconcile_preserves_single_tracked_observation_pump() {
    let attachment_id = AttachmentId::from(8);
    let current = attachment(8, "before");
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
    let old_sender = runtime.inner.i_am_pumps.lock().await[&attachment_id]
        .cancel
        .clone();
    let (old_cov_sender, old_cov_token) = {
        let pumps = runtime.inner.cov_pumps.lock().await;
        let control = &pumps[&attachment_id];
        (control.cancel.clone(), control.pump_token)
    };
    let old_task_count = runtime.inner.supervisor.task_count();

    let mut relabeled = current;
    relabeled.label = "after".to_owned();
    let report = runtime.reconcile(2, vec![relabeled.clone()]).await.unwrap();
    assert_eq!(report.updated, vec![attachment_id]);
    tokio::task::yield_now().await;
    relabeled.label = "after-again".to_owned();
    let report = runtime.reconcile(3, vec![relabeled]).await.unwrap();
    assert_eq!(report.updated, vec![attachment_id]);
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    let new_epoch = runtime
        .inner
        .registry
        .read()
        .await
        .attachment_epoch(attachment_id)
        .unwrap();
    let pumps = runtime.inner.i_am_pumps.lock().await;
    let new_control = &pumps[&attachment_id];
    assert_eq!(new_epoch, old_epoch);
    assert_eq!(new_control.attachment_epoch, old_epoch);
    assert!(old_sender.same_channel(&new_control.cancel));
    drop(pumps);

    let pumps = runtime.inner.cov_pumps.lock().await;
    let new_cov_control = &pumps[&attachment_id];
    assert_eq!(new_cov_control.attachment_epoch, old_epoch);
    assert_eq!(new_cov_control.pump_token, old_cov_token);
    assert!(old_cov_sender.same_channel(&new_cov_control.cancel));
    assert_eq!(pumps.len(), 1);
    drop(pumps);
    assert_eq!(runtime.inner.supervisor.task_count(), old_task_count);

    // Clear lifecycle events, then send one wire notification. If a
    // metadata reconcile stranded an untracked successor, the later
    // reconcile above would install another receiver and duplicate this.
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
            &unsolicited_cov_apdu(80_008, 400_008),
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
                        if notification.subscriber_process_identifier == 80_008
                )
            }) {
                break events;
            }
        }
    })
    .await
    .expect("label-preserved COV pump did not deliver");
    let mut matching = first
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::UnsolicitedCovNotification { ref notification }
                    if notification.subscriber_process_identifier == 80_008
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
                    if notification.subscriber_process_identifier == 80_008
            )
        })
        .count();
    assert_eq!(matching, 1);
    assert_eq!(runtime.inner.cov_pumps.lock().await.len(), 1);
    assert_eq!(runtime.inner.supervisor.task_count(), old_task_count);

    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
    assert!(runtime.inner.cov_pumps.lock().await.is_empty());
    assert_eq!(runtime.inner.supervisor.task_count(), 0);
}

