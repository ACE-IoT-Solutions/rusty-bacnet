use super::*;

impl BacnetRuntime {
    /// Executes an APDU-aware property scan and bounded RP fallback entirely in Rust.
    pub async fn scan(&self, request: ScanRequest) -> Result<ScanSnapshot, RuntimeError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        if request.progress_interval.is_zero() {
            return Err(RuntimeError::invalid_config(
                "scan progress_interval must be non-zero",
            ));
        }
        let started = tokio::time::Instant::now();
        let observation = self
            .inner
            .device_index
            .read()
            .await
            .get(request.device)
            .cloned()
            .ok_or_else(|| RuntimeError::device_not_found(request.device))?;
        let capabilities = self.inner.capabilities.read().await.get(request.device);
        let batches = ScanPlanner::plan_rpm(&request.reads, request.limits)?;
        let total_batches = batches.len();
        // The registry read guard is a transport lifecycle lease: keep it
        // through I/O and cache updates so reconciliation cannot retire this
        // attachment and then receive stale results. State snapshots must
        // release their guards before awaiting this lease (see health/batch).
        let registry = self.inner.registry.read().await;
        let transport = registry.transport(request.device.attachment_id)?;
        let mut outcomes = Vec::with_capacity(request.reads.len());
        let mut errors = Vec::new();
        let mut rpm_attempts = 0;
        let mut rp_fallbacks = 0;
        let mut last_progress = started;

        for (batch_index, batch) in batches.into_iter().enumerate() {
            let rpm_result = if capabilities.rpm == Support::Unsupported {
                None
            } else {
                rpm_attempts += 1;
                Some(
                    transport
                        .read_rpm(request.device.attachment_id, &observation.path, &batch)
                        .await,
                )
            };
            match rpm_result {
                Some(Ok(ack)) => {
                    self.inner
                        .capabilities
                        .write()
                        .await
                        .record_rpm_success(request.device, batch.properties.len());
                    outcomes.extend(ScanExecutor::reconcile_rpm(&batch, ack));
                }
                failed => {
                    if let Some(Err(error)) = failed {
                        if error.code == crate::ErrorCode::Reject {
                            self.inner
                                .capabilities
                                .write()
                                .await
                                .record_rpm_unsupported(request.device);
                        }
                        errors.push(error);
                    }
                    for planned in &batch.properties {
                        rp_fallbacks += 1;
                        let outcome = match transport
                            .read_one(
                                request.device.attachment_id,
                                &observation.path,
                                &planned.read,
                            )
                            .await
                        {
                            Ok(value) => ScanExecutor::rp_success(&planned.read, value),
                            Err(error) => ScanExecutor::operation_failure(&planned.read, &error),
                        };
                        outcomes.push(outcome);
                    }
                }
            }
            let completed_batches = batch_index + 1;
            let final_update = completed_batches == total_batches;
            if final_update || last_progress.elapsed() >= request.progress_interval {
                self.inner
                    .events
                    .publish(
                        self.inner.generation.load(Ordering::Acquire),
                        Some(request.device.attachment_id),
                        EventKind::ScanProgress {
                            device_instance: request.device.device_instance,
                            completed_batches,
                            total_batches,
                            final_update,
                        },
                    )
                    .await;
                last_progress = tokio::time::Instant::now();
            }
        }
        outcomes.sort_by_key(|outcome| outcome.input_index);
        let observed_at = std::time::Instant::now();
        {
            let mut values = self.inner.values.write().await;
            for outcome in &outcomes {
                if let Some(raw_value) = &outcome.raw_value {
                    values.insert(
                        request.device,
                        &outcome.read,
                        raw_value.clone(),
                        observed_at,
                    );
                }
            }
        }
        Ok(ScanSnapshot {
            generation: self.inner.generation.load(Ordering::Acquire),
            device: request.device,
            path: observation.path,
            outcomes,
            errors,
            rpm_attempts,
            rp_fallbacks,
            elapsed: started.elapsed(),
        })
    }

    /// Enumerates the Device object-list and expands the reviewed property catalog in Rust.
    pub async fn scan_device(
        &self,
        request: DeviceScanRequest,
    ) -> Result<DeviceScanSnapshot, RuntimeError> {
        if request.max_objects == 0 {
            return Err(RuntimeError::invalid_config("max_objects must be non-zero"));
        }
        let objects = self
            .enumerate_objects(request.device, request.max_objects)
            .await?;
        let reads = ScanPlanner::catalog_reads(&request.catalog, &objects);
        let properties = self
            .scan(ScanRequest {
                device: request.device,
                reads,
                limits: request.limits,
                progress_interval: request.progress_interval,
            })
            .await?;
        Ok(DeviceScanSnapshot {
            objects,
            properties,
        })
    }

    pub(super) async fn enumerate_objects(
        &self,
        device: crate::DeviceKey,
        max_objects: usize,
    ) -> Result<Vec<(u16, u32)>, RuntimeError> {
        let observation = self
            .inner
            .device_index
            .read()
            .await
            .get(device)
            .cloned()
            .ok_or_else(|| RuntimeError::device_not_found(device))?;
        let attempts = self
            .inner
            .capabilities
            .read()
            .await
            .object_list_attempts(device);
        // As in scan, registry precedes capability writes; no capability or
        // device-index guard may survive into this transport lifecycle lease.
        let registry = self.inner.registry.read().await;
        let transport = registry.transport(device.attachment_id)?;
        let whole = crate::PropertyRead {
            input_index: 0,
            object_type: 8,
            object_instance: device.device_instance,
            property_id: 76,
            array_index: None,
            value_category: "object-identifier-list".to_owned(),
        };
        let mut last_error = None;
        for attempt in attempts {
            let result = match attempt {
                ObjectListAttempt::RpmWholeArray => {
                    let batches = ScanPlanner::plan_rpm(
                        std::slice::from_ref(&whole),
                        crate::PlanLimits {
                            max_request_bytes: 1476,
                            max_estimated_response_bytes: 65_535,
                            max_properties: 1,
                        },
                    )?;
                    match transport
                        .read_rpm(device.attachment_id, &observation.path, &batches[0])
                        .await
                    {
                        Ok(ack) => ScanExecutor::reconcile_rpm(&batches[0], ack)
                            .into_iter()
                            .next()
                            .and_then(|outcome| outcome.raw_value)
                            .ok_or_else(|| {
                                RuntimeError::decode("RPM object-list returned no value")
                            }),
                        Err(error) => Err(error),
                    }
                    .and_then(|raw| decode_object_list(&raw, max_objects))
                }
                ObjectListAttempt::RpWholeArray => transport
                    .read_one(device.attachment_id, &observation.path, &whole)
                    .await
                    .and_then(|raw| decode_object_list(&raw, max_objects)),
                ObjectListAttempt::IndexedCount => {
                    enumerate_indexed(transport, device, &observation.path, &whole, max_objects)
                        .await
                }
            };
            match result {
                Ok(objects) => {
                    let strategy = match attempt {
                        ObjectListAttempt::RpmWholeArray => ObjectListStrategy::RpmWholeArray,
                        ObjectListAttempt::RpWholeArray => ObjectListStrategy::RpWholeArray,
                        ObjectListAttempt::IndexedCount => ObjectListStrategy::Indexed,
                    };
                    self.inner
                        .capabilities
                        .write()
                        .await
                        .record_object_list_success(device, strategy);
                    return Ok(objects);
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error
            .unwrap_or_else(|| RuntimeError::decode("object-list fallback produced no result")))
    }
}
