use super::*;

impl BacnetRuntime {
    /// Submits a read batch and returns its cancellable operation identity immediately.
    pub async fn submit_read_batch(
        &self,
        request: ReadBatch,
    ) -> Result<ReadBatchOperation, RuntimeError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        let priority = request.priority;
        let deadline = request.deadline;
        let (response, receiver) = oneshot::channel();
        let id = self.inner.work_queue.lock().await.enqueue(
            priority,
            deadline,
            RuntimeCommand::Read {
                request,
                response: Some(response),
            },
        )?;
        self.inner.work_notify.notify_one();
        Ok(ReadBatchOperation::new(id, receiver))
    }

    /// Submits a write batch and returns its cancellable operation identity immediately.
    pub async fn submit_write_batch(
        &self,
        request: WriteBatch,
    ) -> Result<WriteBatchOperation, RuntimeError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        let priority = request.priority;
        let deadline = request.deadline;
        let (response, receiver) = oneshot::channel();
        let id = self.inner.work_queue.lock().await.enqueue(
            priority,
            deadline,
            RuntimeCommand::Write {
                request,
                response: Some(response),
            },
        )?;
        self.inner.work_notify.notify_one();
        Ok(WriteBatchOperation::new(id, receiver))
    }

    /// Cancels queued work or signals cooperative cancellation to an in-flight operation.
    pub async fn cancel(&self, id: OperationId) -> CancelReport {
        if let Some(scheduled) = self.inner.work_queue.lock().await.cancel(id) {
            scheduled.payload.fail(RuntimeError::cancelled(
                "operation cancelled before dispatch",
            ));
            return CancelReport {
                id,
                queued: true,
                in_flight: false,
            };
        }
        let in_flight = self
            .inner
            .inflight
            .lock()
            .await
            .get(&id)
            .is_some_and(|cancel| cancel.send(true).is_ok());
        CancelReport {
            id,
            queued: false,
            in_flight,
        }
    }

    /// Schedules and awaits an ordered native read batch.
    pub async fn read_batch(&self, request: ReadBatch) -> Result<Vec<ReadOutcome>, RuntimeError> {
        self.submit_read_batch(request).await?.result().await
    }

    /// Schedules and awaits an ordered native write batch.
    pub async fn write_batch(
        &self,
        request: WriteBatch,
    ) -> Result<Vec<WriteOutcome>, RuntimeError> {
        self.submit_write_batch(request).await?.result().await
    }

    /// Executes attachment-qualified reads concurrently and returns input-index order.
    pub(super) async fn execute_read_batch(
        &self,
        request: ReadBatch,
    ) -> Result<Vec<ReadOutcome>, RuntimeError> {
        validate_unique_indices(
            request.items.iter().map(|item| item.input_index),
            "read batch",
        )?;
        let mut grouped = std::collections::BTreeMap::new();
        let mut outcomes = Vec::new();
        let now = std::time::Instant::now();
        for item in request.items {
            if let FreshnessPolicy::MaxAge(max_age) = item.freshness {
                // An if-let scrutinee guard lives through its body in Rust
                // 2021. Own the cached value before awaiting the device index:
                // reconciliation takes the index before clearing the cache.
                let cached =
                    self.inner
                        .values
                        .read()
                        .await
                        .get_fresh(item.device, &item.read, max_age, now);
                if let Some(value) = cached {
                    let path = self
                        .inner
                        .device_index
                        .read()
                        .await
                        .get(item.device)
                        .map(|observation| observation.path.clone());
                    outcomes.push(ReadOutcome {
                        input_index: item.input_index,
                        device: item.device,
                        path,
                        raw_value: Some(value.raw_value),
                        source: Some(ReadSource::Cache),
                        error: None,
                    });
                    continue;
                }
            }
            grouped
                .entry(item.device)
                .or_insert_with(Vec::new)
                .push(item);
        }
        let futures = grouped.into_iter().map(|(device, items)| async move {
            let reads = items
                .iter()
                .map(|item| {
                    let mut read = item.read.clone();
                    read.input_index = item.input_index;
                    read
                })
                .collect::<Vec<_>>();
            let operation = self.scan(ScanRequest {
                device,
                reads,
                limits: crate::PlanLimits {
                    max_request_bytes: 1476,
                    max_estimated_response_bytes: 1476,
                    max_properties: 32,
                },
                progress_interval: Duration::from_secs(3600),
            });
            let outcomes: Vec<ReadOutcome> = match tokio::time::timeout_at(
                tokio::time::Instant::from_std(request.deadline),
                operation,
            )
            .await
            {
                Ok(Ok(snapshot)) => snapshot
                    .outcomes
                    .into_iter()
                    .map(|outcome| ReadOutcome {
                        input_index: outcome.input_index,
                        device,
                        path: Some(snapshot.path.clone()),
                        source: outcome.raw_value.as_ref().map(|_| ReadSource::Wire),
                        raw_value: outcome.raw_value,
                        error: outcome.error,
                    })
                    .collect(),
                Ok(Err(error)) => items
                    .into_iter()
                    .map(|item| read_failure(item.input_index, device, &error))
                    .collect(),
                Err(_) => {
                    let error =
                        RuntimeError::deadline_exceeded("read batch device deadline elapsed");
                    items
                        .into_iter()
                        .map(|item| read_failure(item.input_index, device, &error))
                        .collect()
                }
            };
            outcomes
        });
        outcomes.extend(join_all(futures).await.into_iter().flatten());
        outcomes.sort_by_key(|outcome| outcome.input_index);
        Ok(outcomes)
    }

    /// Executes pre-authorized writes concurrently and returns input-index order.
    pub(super) async fn execute_write_batch(
        &self,
        request: WriteBatch,
    ) -> Result<Vec<WriteOutcome>, RuntimeError> {
        validate_unique_indices(
            request.items.iter().map(|item| item.input_index),
            "write batch",
        )?;
        for item in &request.items {
            if item.authorization_id.is_empty() {
                return Err(RuntimeError::invalid_config(
                    "write authorization_id must not be empty",
                ));
            }
            if !matches!(item.bacnet_priority, None | Some(1..=16)) {
                return Err(RuntimeError::invalid_config(
                    "BACnet write priority must be between 1 and 16",
                ));
            }
        }
        let deadline = request.deadline;
        let mut groups: BTreeMap<(crate::DeviceKey, bool), Vec<WriteBatchItem>> = BTreeMap::new();
        for item in request.items {
            // Verified writes retain their individual WP + readback exchange;
            // ordinary writes to one device share one WPM request.
            groups
                .entry((item.device, item.verify_readback))
                .or_default()
                .push(item);
        }
        let futures = groups.into_values().map(|items| async move {
            if items.len() == 1 || items[0].verify_readback {
                let mut outcomes = Vec::with_capacity(items.len());
                for item in items {
                    outcomes.push(self.execute_write_item(item, deadline).await);
                }
                outcomes
            } else {
                self.execute_write_group(items, deadline).await
            }
        });
        let mut outcomes: Vec<WriteOutcome> =
            join_all(futures).await.into_iter().flatten().collect();
        outcomes.sort_by_key(|outcome| outcome.input_index);
        Ok(outcomes)
    }

    pub(super) async fn execute_write_group(
        &self,
        items: Vec<WriteBatchItem>,
        deadline: std::time::Instant,
    ) -> Vec<WriteOutcome> {
        let device = items[0].device;
        let observation = match self.inner.device_index.read().await.get(device).cloned() {
            Some(observation) => observation,
            None => {
                let error = RuntimeError::device_not_found(device);
                return items
                    .into_iter()
                    .map(|item| WriteOutcome {
                        input_index: item.input_index,
                        device: item.device,
                        path: None,
                        authorization_id: item.authorization_id,
                        written: false,
                        readback: None,
                        error: Some(crate::batch::outcome_error(&error)),
                    })
                    .collect();
            }
        };
        let registry = self.inner.registry.read().await;
        let transport = match registry.transport(device.attachment_id) {
            Ok(transport) => transport,
            Err(error) => {
                return items
                    .into_iter()
                    .map(|item| WriteOutcome {
                        input_index: item.input_index,
                        device: item.device,
                        path: Some(observation.path.clone()),
                        authorization_id: item.authorization_id,
                        written: false,
                        readback: None,
                        error: Some(crate::batch::outcome_error(&error)),
                    })
                    .collect();
            }
        };
        let writes = items
            .iter()
            .map(|item| {
                (
                    item.target.clone(),
                    item.raw_value.clone(),
                    item.bacnet_priority,
                )
            })
            .collect::<Vec<_>>();
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            transport.write_multiple(device.attachment_id, &observation.path, &writes),
        )
        .await;
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some(RuntimeError::deadline_exceeded(
                "write batch deadline elapsed",
            )),
        };
        if error.is_none() {
            let mut cache = self.inner.values.write().await;
            for item in &items {
                cache.remove(item.device, &item.target);
            }
        }
        items
            .into_iter()
            .map(|item| WriteOutcome {
                input_index: item.input_index,
                device: item.device,
                path: Some(observation.path.clone()),
                authorization_id: item.authorization_id,
                written: error.is_none(),
                readback: None,
                error: error.as_ref().map(crate::batch::outcome_error),
            })
            .collect()
    }

    pub(super) async fn execute_write_item(
        &self,
        item: WriteBatchItem,
        deadline: std::time::Instant,
    ) -> WriteOutcome {
        let base = |path, written, readback, error| WriteOutcome {
            input_index: item.input_index,
            device: item.device,
            path,
            authorization_id: item.authorization_id.clone(),
            written,
            readback,
            error,
        };
        let observation = match self
            .inner
            .device_index
            .read()
            .await
            .get(item.device)
            .cloned()
        {
            Some(observation) => observation,
            None => {
                let error = RuntimeError::device_not_found(item.device);
                return base(None, false, None, Some(crate::batch::outcome_error(&error)));
            }
        };
        let registry = self.inner.registry.read().await;
        let transport = match registry.transport(item.device.attachment_id) {
            Ok(transport) => transport,
            Err(error) => {
                return base(
                    Some(observation.path),
                    false,
                    None,
                    Some(crate::batch::outcome_error(&error)),
                );
            }
        };
        let write = transport.write_one(
            item.device.attachment_id,
            &observation.path,
            &item.target,
            item.raw_value.clone(),
            item.bacnet_priority,
        );
        match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), write).await {
            Err(_) => {
                let error = RuntimeError::deadline_exceeded("write deadline elapsed");
                base(
                    Some(observation.path),
                    false,
                    None,
                    Some(crate::batch::outcome_error(&error)),
                )
            }
            Ok(Err(error)) => base(
                Some(observation.path),
                false,
                None,
                Some(crate::batch::outcome_error(&error)),
            ),
            Ok(Ok(())) if !item.verify_readback => {
                // A successful priority write does not prove the resulting
                // Present_Value (a higher command priority may still win), so
                // invalidate rather than caching the requested value.
                self.inner
                    .values
                    .write()
                    .await
                    .remove(item.device, &item.target);
                base(Some(observation.path), true, None, None)
            }
            Ok(Ok(())) => {
                self.inner
                    .values
                    .write()
                    .await
                    .remove(item.device, &item.target);
                let readback =
                    transport.read_one(item.device.attachment_id, &observation.path, &item.target);
                match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), readback)
                    .await
                {
                    Ok(Ok(value)) => {
                        self.inner.values.write().await.insert(
                            item.device,
                            &item.target,
                            value.clone(),
                            std::time::Instant::now(),
                        );
                        if value == item.raw_value {
                            base(Some(observation.path), true, Some(value), None)
                        } else {
                            let error = RuntimeError::property_error(
                                "write readback did not match requested raw value",
                            );
                            base(
                                Some(observation.path),
                                true,
                                Some(value),
                                Some(crate::batch::outcome_error(&error)),
                            )
                        }
                    }
                    Ok(Err(error)) => base(
                        Some(observation.path),
                        true,
                        None,
                        Some(crate::batch::outcome_error(&error)),
                    ),
                    Err(_) => {
                        let error =
                            RuntimeError::deadline_exceeded("write readback deadline elapsed");
                        base(
                            Some(observation.path),
                            true,
                            None,
                            Some(crate::batch::outcome_error(&error)),
                        )
                    }
                }
            }
        }
    }
}
