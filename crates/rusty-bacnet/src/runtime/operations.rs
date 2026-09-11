use super::*;

#[pymethods]
impl PyBACnetRuntime {
    /// Executes an ordered, attachment-qualified read batch in one Python/Rust crossing.
    #[pyo3(signature = (reads, timeout_ms=5000, priority="foreground"))]
    fn read_batch<'py>(
        &self,
        py: Python<'py>,
        reads: Vec<PyRuntimeRead>,
        timeout_ms: u64,
        priority: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = parse_priority(priority)?;
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(reads.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let items = reads
                .into_iter()
                .map(|read| ReadBatchItem {
                    input_index: read.input_index,
                    device: device_key(read.attachment_id, read.device_instance),
                    read: PropertyRead {
                        input_index: read.input_index,
                        object_type: read.object_type,
                        object_instance: read.object_instance,
                        property_id: read.property_id,
                        array_index: read.array_index,
                        value_category: read.value_category,
                    },
                    freshness: read.max_age_ms.map_or(FreshnessPolicy::WireOnly, |age| {
                        FreshnessPolicy::MaxAge(Duration::from_millis(age))
                    }),
                })
                .collect();
            let outcomes = runtime
                .read_batch(ReadBatch {
                    items,
                    deadline: Instant::now() + Duration::from_millis(timeout_ms),
                    priority,
                })
                .await
                .map_err(runtime_error)?;
            crossings
                .output_items
                .fetch_add(outcomes.len() as u64, Ordering::Relaxed);
            Ok(outcomes
                .into_iter()
                .map(read_outcome_to_py)
                .collect::<Vec<_>>())
        })
    }

    /// Submits a cancellable read batch and returns its operation ID before result wait.
    #[pyo3(signature = (reads, timeout_ms=5000, priority="foreground"))]
    fn submit_read_batch<'py>(
        &self,
        py: Python<'py>,
        reads: Vec<PyRuntimeRead>,
        timeout_ms: u64,
        priority: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = parse_priority(priority)?;
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(reads.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let operation = runtime
                .submit_read_batch(read_batch_request(reads, timeout_ms, priority))
                .await
                .map_err(runtime_error)?;
            Ok(PyRuntimeReadOperation {
                operation_id: operation.id.0,
                inner: Arc::new(tokio::sync::Mutex::new(Some(operation))),
                crossings,
            })
        })
    }

    /// Executes a pre-authorized ordered write batch in one Python/Rust crossing.
    #[pyo3(signature = (writes, timeout_ms=5000, priority="foreground"))]
    fn write_batch<'py>(
        &self,
        py: Python<'py>,
        writes: Vec<PyRuntimeWrite>,
        timeout_ms: u64,
        priority: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = parse_priority(priority)?;
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(writes.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let items = writes
                .into_iter()
                .map(|write| WriteBatchItem {
                    input_index: write.input_index,
                    device: device_key(write.attachment_id, write.device_instance),
                    target: PropertyRead {
                        input_index: write.input_index,
                        object_type: write.object_type,
                        object_instance: write.object_instance,
                        property_id: write.property_id,
                        array_index: write.array_index,
                        value_category: String::new(),
                    },
                    raw_value: write.raw_value,
                    bacnet_priority: write.bacnet_priority,
                    authorization_id: write.authorization_id,
                    verify_readback: write.verify_readback,
                })
                .collect();
            let outcomes = runtime
                .write_batch(WriteBatch {
                    items,
                    deadline: Instant::now() + Duration::from_millis(timeout_ms),
                    priority,
                })
                .await
                .map_err(runtime_error)?;
            crossings
                .output_items
                .fetch_add(outcomes.len() as u64, Ordering::Relaxed);
            Ok(outcomes
                .into_iter()
                .map(write_outcome_to_py)
                .collect::<Vec<_>>())
        })
    }

    /// Submits a cancellable write batch and returns its operation ID before result wait.
    #[pyo3(signature = (writes, timeout_ms=5000, priority="foreground"))]
    fn submit_write_batch<'py>(
        &self,
        py: Python<'py>,
        writes: Vec<PyRuntimeWrite>,
        timeout_ms: u64,
        priority: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let priority = parse_priority(priority)?;
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(writes.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let operation = runtime
                .submit_write_batch(write_batch_request(writes, timeout_ms, priority))
                .await
                .map_err(runtime_error)?;
            Ok(PyRuntimeWriteOperation {
                operation_id: operation.id.0,
                inner: Arc::new(tokio::sync::Mutex::new(Some(operation))),
                crossings,
            })
        })
    }

    /// Applies a complete revisioned managed-COV plan transactionally.
    fn apply_observation_plan<'py>(
        &self,
        py: Python<'py>,
        revision: u64,
        subscriptions: Vec<PyRuntimeObservation>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(subscriptions.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let subscriptions = subscriptions
                .into_iter()
                .map(|item| ObservationSpec {
                    key: ObservationKey {
                        device: device_key(item.attachment_id, item.device_instance),
                        object_type: item.object_type,
                        object_instance: item.object_instance,
                        subscriber_process_id: item.subscriber_process_id,
                    },
                    confirmed: item.confirmed,
                    lifetime_seconds: item.lifetime_seconds,
                    renewal_margin: Duration::from_secs(item.renewal_margin_seconds),
                    suppress_poll_when_fresh: item.suppress_poll_when_fresh,
                })
                .collect();
            let report = runtime
                .apply_observation_plan(ObservationPlan {
                    revision,
                    subscriptions,
                })
                .await
                .map_err(runtime_error)?;
            Ok(PyRuntimeObservationReport {
                revision: report.revision,
                added: report.added.len(),
                updated: report.updated.len(),
                removed: report.removed.len(),
                idempotent: report.idempotent,
            })
        })
    }

    /// Requests cancellation for a previously returned native operation identity.
    fn cancel<'py>(&self, py: Python<'py>, operation_id: u64) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let report = runtime.cancel(OperationId(operation_id)).await;
            Ok(PyRuntimeCancelReport {
                operation_id: report.id.0,
                queued: report.queued,
                in_flight: report.in_flight,
                found: report.found(),
            })
        })
    }

    /// Retrieves a bounded typed event batch, including explicit lag events.
    #[pyo3(signature = (max_items=256, wait_ms=0))]
    fn next_events<'py>(
        &self,
        py: Python<'py>,
        max_items: usize,
        wait_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let batch = runtime
                .next_events(max_items, Duration::from_millis(wait_ms))
                .await;
            crossings
                .output_items
                .fetch_add(batch.events.len() as u64, Ordering::Relaxed);
            Ok(PyRuntimeEventBatch {
                events: batch.events.into_iter().map(event_to_py).collect(),
            })
        })
    }

    /// Stops every owned attachment and supervised task.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            runtime.stop().await.map_err(runtime_error)?;
            Ok(())
        })
    }
}
