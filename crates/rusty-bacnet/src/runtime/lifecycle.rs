use super::*;

#[pymethods]
impl PyBACnetRuntime {
    /// Starts typed B/IP and MS/TP attachments under one native supervisor.
    #[classmethod]
    #[pyo3(signature = (attachments, event_capacity=1024, max_event_batch=256))]
    fn start<'py>(
        _class: &Bound<'py, PyType>,
        py: Python<'py>,
        attachments: Vec<PyRuntimeAttachmentConfig>,
        event_capacity: usize,
        max_event_batch: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        if attachments.is_empty() {
            return Err(PyValueError::new_err("at least one attachment is required"));
        }
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let attachments = attachments
                .into_iter()
                .map(PyRuntimeAttachmentConfig::into_rust)
                .collect();
            let runtime = BacnetRuntime::start(RuntimeConfig {
                attachments,
                event_capacity,
                max_event_batch,
                ..RuntimeConfig::default()
            })
            .await
            .map_err(runtime_error)?;
            Ok(Self {
                inner: runtime,
                crossings: Arc::new(CrossingCounters::default()),
                catalog: Arc::new(tokio::sync::RwLock::new(None)),
            })
        })
    }

    /// Transactionally applies a complete revisioned attachment configuration.
    fn reconcile<'py>(
        &self,
        py: Python<'py>,
        revision: u64,
        attachments: Vec<PyRuntimeAttachmentConfig>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(attachments.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let report = runtime
                .reconcile(
                    revision,
                    attachments
                        .into_iter()
                        .map(PyRuntimeAttachmentConfig::into_rust)
                        .collect(),
                )
                .await
                .map_err(runtime_error)?;
            let output_items = report.added.len() + report.updated.len() + report.removed.len();
            crossings
                .output_items
                .fetch_add(output_items as u64, Ordering::Relaxed);
            Ok(PyRuntimeReconcileReport {
                revision: report.revision,
                generation: report.generation,
                added: report.added.into_iter().map(attachment_u128).collect(),
                updated: report.updated.into_iter().map(attachment_u128).collect(),
                removed: report.removed.into_iter().map(attachment_u128).collect(),
                idempotent: report.idempotent,
            })
        })
    }

    /// Restores persisted transport-native paths without running discovery.
    fn restore_devices<'py>(
        &self,
        py: Python<'py>,
        devices: Vec<PyRuntimePersistedDevice>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(devices.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let report = runtime
                .restore_devices(
                    devices
                        .into_iter()
                        .map(PyRuntimePersistedDevice::into_rust)
                        .collect(),
                )
                .await
                .map_err(runtime_error)?;
            let output_items = report.added + report.updated + report.unchanged;
            crossings
                .output_items
                .fetch_add(output_items as u64, Ordering::Relaxed);
            Ok(PyRuntimeDeviceRestoreReport {
                added: report.added,
                updated: report.updated,
                unchanged: report.unchanged,
                index_revision: report.index_revision,
            })
        })
    }

    /// Discovers devices and routers with exact attachment/path provenance.
    #[pyo3(signature = (timeout_ms=1000, low_limit=None, high_limit=None))]
    fn discover<'py>(
        &self,
        py: Python<'py>,
        timeout_ms: u64,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let snapshot = runtime
                .discover(DiscoveryRequest {
                    low_limit,
                    high_limit,
                    observation_window: Duration::from_millis(timeout_ms),
                    ..DiscoveryRequest::default()
                })
                .await
                .map_err(runtime_error)?;
            let devices = snapshot
                .devices
                .into_iter()
                .flat_map(|selection| {
                    selection
                        .selected
                        .into_iter()
                        .map(|item| (item, true))
                        .chain(selection.alternates.into_iter().map(|item| (item, false)))
                        .map(|(observation, selected)| observation_to_py(observation, selected))
                })
                .collect::<Vec<_>>();
            let routers = snapshot
                .routers
                .into_iter()
                .map(router_to_py)
                .collect::<Vec<_>>();
            crossings
                .output_items
                .fetch_add((devices.len() + routers.len()) as u64, Ordering::Relaxed);
            Ok(PyRuntimeDiscoverySnapshot {
                generation: snapshot.generation,
                index_revision: snapshot.index_revision,
                devices,
                routers,
                error_codes: snapshot
                    .errors
                    .into_iter()
                    .map(|error| format!("{:?}", error.code))
                    .collect(),
            })
        })
    }

    /// Walks router and BBMD/FDT topology inside the native runtime.
    #[pyo3(signature = (attachment_ids=Vec::new(), bbmd_seeds=Vec::new(), include_fdt=true, max_bbmds_per_attachment=256))]
    fn topology<'py>(
        &self,
        py: Python<'py>,
        attachment_ids: Vec<u128>,
        bbmd_seeds: Vec<(u128, Vec<u8>)>,
        include_fdt: bool,
        max_bbmds_per_attachment: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings.input_items.fetch_add(
            (attachment_ids.len() + bbmd_seeds.len()) as u64,
            Ordering::Relaxed,
        );
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let snapshot = runtime
                .topology(TopologyRequest {
                    attachment_ids: attachment_ids.into_iter().map(AttachmentId::from).collect(),
                    bbmd_seeds: bbmd_seeds
                        .into_iter()
                        .map(|(attachment_id, mac)| BbmdTarget {
                            attachment_id: AttachmentId::from(attachment_id),
                            mac,
                        })
                        .collect(),
                    include_fdt,
                    max_bbmds_per_attachment,
                })
                .await
                .map_err(runtime_error)?;
            let attachments = snapshot
                .attachments
                .into_iter()
                .map(topology_attachment_to_py)
                .collect::<Vec<_>>();
            crossings
                .output_items
                .fetch_add(attachments.len() as u64, Ordering::Relaxed);
            Ok(PyRuntimeTopologySnapshot {
                generation: snapshot.generation,
                attachments,
                error_codes: snapshot
                    .errors
                    .into_iter()
                    .map(|error| format!("{:?}", error.code))
                    .collect(),
            })
        })
    }

    /// Executes all supplied properties inside one Rust scan operation.
    #[pyo3(signature = (attachment_id, device_instance, reads, max_request_bytes=1476, max_response_bytes=1476, max_properties=32, progress_interval_ms=250))]
    #[allow(clippy::too_many_arguments)]
    fn scan<'py>(
        &self,
        py: Python<'py>,
        attachment_id: u128,
        device_instance: u32,
        reads: Vec<ScanRead>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        max_properties: usize,
        progress_interval_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        crossings
            .input_items
            .fetch_add(reads.len() as u64, Ordering::Relaxed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let snapshot = runtime
                .scan(ScanRequest {
                    device: bacnet_runtime::DeviceKey {
                        attachment_id: AttachmentId::from(attachment_id),
                        device_instance,
                    },
                    reads: reads
                        .into_iter()
                        .map(
                            |(
                                input_index,
                                object_type,
                                object_instance,
                                property_id,
                                array_index,
                                value_category,
                            )| PropertyRead {
                                input_index,
                                object_type,
                                object_instance,
                                property_id,
                                array_index,
                                value_category,
                            },
                        )
                        .collect(),
                    limits: PlanLimits {
                        max_request_bytes,
                        max_estimated_response_bytes: max_response_bytes,
                        max_properties,
                    },
                    progress_interval: Duration::from_millis(progress_interval_ms),
                })
                .await
                .map_err(runtime_error)?;
            crossings
                .output_items
                .fetch_add(snapshot.outcomes.len() as u64, Ordering::Relaxed);
            Ok(snapshot_to_py(snapshot))
        })
    }

    /// Validates and installs one immutable property catalog for device scans.
    fn load_property_catalog<'py>(
        &self,
        py: Python<'py>,
        catalog_json: Vec<u8>,
        version: u32,
        sha256: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let catalog_slot = Arc::clone(&self.catalog);
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            crossings.input_items.fetch_add(1, Ordering::Relaxed);
            let catalog =
                PropertyCatalog::load(&catalog_json, version, &sha256).map_err(runtime_error)?;
            *catalog_slot.write().await = Some(catalog);
            Ok(())
        })
    }

    /// Enumerates object-list and enriches every catalog property inside Rust.
    #[pyo3(signature = (attachment_id, device_instance, max_objects=50000, max_request_bytes=1476, max_response_bytes=1476, max_properties=32, progress_interval_ms=250))]
    #[allow(clippy::too_many_arguments)]
    fn scan_device<'py>(
        &self,
        py: Python<'py>,
        attachment_id: u128,
        device_instance: u32,
        max_objects: usize,
        max_request_bytes: usize,
        max_response_bytes: usize,
        max_properties: usize,
        progress_interval_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let catalog_slot = Arc::clone(&self.catalog);
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let catalog = catalog_slot.read().await.clone().ok_or_else(|| {
                PyRuntimeError::new_err("property catalog must be loaded before scan_device")
            })?;
            let snapshot = runtime
                .scan_device(DeviceScanRequest {
                    device: device_key(attachment_id, device_instance),
                    catalog,
                    max_objects,
                    limits: PlanLimits {
                        max_request_bytes,
                        max_estimated_response_bytes: max_response_bytes,
                        max_properties,
                    },
                    progress_interval: Duration::from_millis(progress_interval_ms),
                })
                .await
                .map_err(runtime_error)?;
            crossings.output_items.fetch_add(
                (snapshot.objects.len() + snapshot.properties.outcomes.len()) as u64,
                Ordering::Relaxed,
            );
            Ok(PyRuntimeDeviceScanSnapshot {
                objects: snapshot
                    .objects
                    .into_iter()
                    .map(|(object_type, object_instance)| PyRuntimeScannedObject {
                        object_type,
                        object_instance,
                    })
                    .collect(),
                properties: snapshot_to_py(snapshot.properties),
            })
        })
    }

    /// Returns typed runtime health without performing network I/O.
    fn health<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let runtime = self.inner.clone();
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            crossings.calls.fetch_add(1, Ordering::Relaxed);
            let health = runtime.health().await;
            Ok(PyRuntimeHealth {
                generation: health.generation,
                accepting_commands: health.accepting_commands,
                task_count: health.task_count,
                supervisor_task_count: health.supervisor_task_count,
                attachment_task_count: health.attachment_task_count,
                event_queue_depth: health.event_queue_depth,
                event_lag_count: health.event_lag_count,
                cov_notification_lag_count: health.cov_notification_lag_count,
                i_am_observation_lag_count: health.i_am_observation_lag_count,
                device_count: health.device_count,
                device_observation_count: health.device_observation_count,
                capability_count: health.capability_count,
                cached_value_count: health.cached_value_count,
                observation_count: health.observation_count,
                attachment_ids: health
                    .attachments
                    .iter()
                    .map(|attachment| attachment_u128(attachment.id))
                    .collect(),
                attachment_states: health
                    .attachments
                    .iter()
                    .map(|attachment| format!("{:?}", attachment.state))
                    .collect(),
                attachment_error_codes: health
                    .attachments
                    .iter()
                    .map(|attachment| attachment.last_error.map(|code| format!("{code:?}")))
                    .collect(),
                foreign_device_statuses: health
                    .attachments
                    .iter()
                    .map(|attachment| {
                        attachment
                            .foreign_device_registration
                            .clone()
                            .map(PyForeignDeviceStatus::from_rust)
                    })
                    .collect(),
            })
        })
    }

    /// Returns current crossing counters without adding a crossing to the count.
    fn crossing_counters(&self) -> PyRuntimeCrossingCounters {
        PyRuntimeCrossingCounters {
            calls: self.crossings.calls.load(Ordering::Relaxed),
            input_items: self.crossings.input_items.load(Ordering::Relaxed),
            output_items: self.crossings.output_items.load(Ordering::Relaxed),
        }
    }
}
