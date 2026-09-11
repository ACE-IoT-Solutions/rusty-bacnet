#[tokio::test]
async fn health_releases_state_locks_behind_queued_registry_writer() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();

    // Reproduce the scan -> reconcile -> health cycle without scheduler
    // timing: scan owns a registry read lease, then a lifecycle writer queues.
    let scan_lease = runtime.inner.registry.read().await;
    let mut lifecycle_writer = Box::pin(runtime.inner.registry.write());
    assert!(futures_util::poll!(&mut lifecycle_writer).is_pending());

    // Tokio's write-preferring registry lock now blocks health. Health must
    // have released every state guard before reaching this suspension point.
    let mut health = Box::pin(runtime.health());
    assert!(futures_util::poll!(&mut health).is_pending());
    let capability_update = timeout(Duration::from_secs(1), runtime.inner.capabilities.write())
        .await
        .expect("health deadlocked a scan's capability update behind a registry writer");
    assert!(runtime.inner.device_index.try_write().is_ok());
    assert!(runtime.inner.values.try_write().is_ok());
    assert!(runtime.inner.observations.try_write().is_ok());
    drop(capability_update);
    drop(scan_lease);

    let lifecycle_guard = timeout(Duration::from_secs(1), lifecycle_writer)
        .await
        .expect("lifecycle writer did not progress after scan released its lease");
    drop(lifecycle_guard);
    let snapshot = timeout(Duration::from_secs(1), health)
        .await
        .expect("health did not progress after lifecycle writer completed");
    assert_eq!(snapshot.device_count, 0);
    assert!(snapshot.attachments.is_empty());
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn cached_batch_releases_values_before_waiting_for_device_index() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let device = DeviceKey {
        attachment_id: AttachmentId::from(901),
        device_instance: 100,
    };
    let path = DevicePath::Direct {
        mac: vec![127, 0, 0, 1, 0xba, 0xc0],
    };
    let read = PropertyRead {
        input_index: 3,
        object_type: 0,
        object_instance: 1,
        property_id: 85,
        array_index: None,
        value_category: "real".to_owned(),
    };
    let raw_value = vec![0x44, 0x42, 0x90, 0, 0];
    runtime
        .inner
        .device_index
        .write()
        .await
        .upsert(DeviceObservation {
            key: device,
            path: path.clone(),
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        });
    runtime.inner.values.write().await.insert(
        device,
        &read,
        raw_value.clone(),
        std::time::Instant::now(),
    );

    // Reconciliation owns the index while clearing attachment caches. Poll
    // the real cached-read path exactly up to its blocked index acquisition.
    let reconcile_index = runtime.inner.device_index.write().await;
    let mut batch = Box::pin(runtime.execute_read_batch(ReadBatch {
        items: vec![ReadBatchItem {
            input_index: 3,
            device,
            read,
            freshness: FreshnessPolicy::MaxAge(Duration::from_secs(60)),
        }],
        deadline: std::time::Instant::now() + Duration::from_secs(5),
        priority: WorkPriority::Foreground,
    }));
    assert!(futures_util::poll!(&mut batch).is_pending());
    let reconcile_values = timeout(Duration::from_secs(1), runtime.inner.values.write())
        .await
        .expect("cached read retained values while awaiting reconciliation's device index");
    drop(reconcile_values);
    drop(reconcile_index);

    let outcomes = timeout(Duration::from_secs(1), batch)
        .await
        .expect("cached batch did not progress after reconciliation released its index")
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].source, Some(ReadSource::Cache));
    assert_eq!(outcomes[0].path, Some(path));
    assert_eq!(outcomes[0].raw_value, Some(raw_value));
    assert!(outcomes[0].error.is_none());
    runtime.stop().await.unwrap();
}
