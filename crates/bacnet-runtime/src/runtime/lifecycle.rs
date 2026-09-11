use super::*;
use crate::{AttachmentHealth, AttachmentState};

fn synchronize_selection_health(index: &mut DeviceIndex, health: &[AttachmentHealth]) {
    for attachment in health {
        index.set_attachment_health(attachment.id, attachment.state == AttachmentState::Running);
    }
}

impl BacnetRuntime {
    /// Validates configuration and starts an independently supervised runtime.
    pub async fn start(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        config.validate()?;
        let registry =
            AttachmentRegistry::start(&config.attachments, config.cov_channel_capacity).await?;
        let mut device_index = DeviceIndex::new(
            config
                .attachments
                .iter()
                .map(|attachment| attachment.id)
                .collect(),
        );
        for attachment in &config.attachments {
            device_index.set_attachment_health(attachment.id, true);
        }
        let work_queue =
            WorkScheduler::new(config.scheduler_capacity, config.max_foreground_burst)?;
        let work_slots = Arc::new(Semaphore::new(config.max_inflight_operations));
        let inner = Arc::new(RuntimeInner {
            generation: AtomicU64::new(1),
            config_revision: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            shutdown_timeout: config.shutdown_timeout,
            registry: RwLock::new(registry),
            device_index: RwLock::new(device_index),
            capabilities: RwLock::new(CapabilityCache::default()),
            values: RwLock::new(ValueCache::default()),
            observations: RwLock::new(ObservationRegistry::default()),
            pending_observations: RwLock::new(std::collections::BTreeMap::new()),
            cov_renewals: Mutex::new(std::collections::BTreeMap::new()),
            cov_pumps: Mutex::new(std::collections::BTreeMap::new()),
            cov_pump_lifecycle: Mutex::new(()),
            next_cov_pump_token: AtomicU64::new(1),
            i_am_pumps: Mutex::new(std::collections::BTreeMap::new()),
            cov_last: Mutex::new(std::collections::BTreeMap::new()),
            cov_notification_lag_count: AtomicU64::new(0),
            i_am_observation_lag_count: AtomicU64::new(0),
            work_queue: Mutex::new(work_queue),
            work_notify: Notify::new(),
            work_slots,
            inflight: Mutex::new(std::collections::BTreeMap::new()),
            events: EventBus::new(config.event_capacity, config.max_event_batch),
            supervisor: Supervisor::new(),
            stop_lock: Mutex::new(()),
            reconcile_lock: Mutex::new(()),
            observation_lock: Mutex::new(()),
        });
        let weak = Arc::downgrade(&inner);
        let spawned = inner
            .supervisor
            .spawn(move |stop| work_loop(weak, stop))
            .await;
        debug_assert!(spawned, "new supervisor must accept its lifecycle task");
        inner
            .events
            .publish(1, None, EventKind::RuntimeStarted)
            .await;
        let runtime = Self { inner };
        for attachment in &config.attachments {
            runtime.ensure_i_am_pump(attachment.id).await?;
            runtime.ensure_cov_pump(attachment.id).await?;
        }
        Ok(runtime)
    }

    /// Returns a health snapshot without performing BACnet I/O.
    ///
    /// Component counters are sampled independently so concurrent updates do
    /// not require holding several runtime locks at once.
    pub async fn health(&self) -> RuntimeHealth {
        // Snapshot each component independently. Retaining a cache/index guard
        // while awaiting the registry can deadlock behind a queued lifecycle
        // writer and an active scan that needs to update that same cache.
        let (device_count, device_observation_count) = {
            let index = self.inner.device_index.read().await;
            (index.device_count(), index.observation_count())
        };
        let capability_count = self.inner.capabilities.read().await.len();
        let cached_value_count = self.inner.values.read().await.len();
        let observation_count = self.inner.observations.read().await.len();
        let (attachment_task_count, attachments) = {
            let registry = self.inner.registry.read().await;
            (registry.background_task_count(), registry.health())
        };
        let supervisor_task_count = self.inner.supervisor.task_count();
        RuntimeHealth {
            generation: self.inner.generation.load(Ordering::Acquire),
            accepting_commands: self.inner.supervisor.accepting()
                && !self.inner.stopped.load(Ordering::Acquire),
            task_count: supervisor_task_count + attachment_task_count,
            supervisor_task_count,
            attachment_task_count,
            event_queue_depth: self.inner.events.queue_depth().await,
            event_lag_count: self.inner.events.lag_count().await,
            cov_notification_lag_count: self
                .inner
                .cov_notification_lag_count
                .load(Ordering::Relaxed),
            i_am_observation_lag_count: self
                .inner
                .i_am_observation_lag_count
                .load(Ordering::Relaxed),
            device_count,
            device_observation_count,
            capability_count,
            cached_value_count,
            observation_count,
            attachments,
        }
    }

    /// Retrieves a bounded event batch or returns empty after `wait`.
    pub async fn next_events(&self, max_items: usize, wait: Duration) -> EventBatch {
        self.inner.events.next_batch(max_items, wait).await
    }

    /// Discovers devices and routers concurrently across selected attachments.
    pub async fn discover(
        &self,
        request: DiscoveryRequest,
    ) -> Result<DiscoverySnapshot, RuntimeError> {
        if request.observation_window.is_zero() {
            return Err(RuntimeError::invalid_config(
                "discovery observation_window must be non-zero",
            ));
        }
        if matches!((request.low_limit, request.high_limit), (Some(low), Some(high)) if low > high)
        {
            return Err(RuntimeError::invalid_config(
                "discovery low_limit must not exceed high_limit",
            ));
        }
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }

        let generation = self.inner.generation.load(Ordering::Acquire);
        // Sample dynamic transport health under the registry guard, then drop
        // that guard before updating the device index. In particular, a
        // disconnected SC attachment must not retain preferred selection over
        // a healthy observation on another attachment.
        let (attachment_results, attachment_health) = {
            let registry = self.inner.registry.read().await;
            let results = registry.discover(&request).await;
            (results, registry.health())
        };
        let mut observed_instances = std::collections::BTreeSet::new();
        let mut routers = Vec::new();
        let mut errors = Vec::new();
        let mut index = self.inner.device_index.write().await;
        synchronize_selection_health(&mut index, &attachment_health);
        for (attachment_id, result) in attachment_results {
            for device in result.devices {
                let observation = device_observation(attachment_id, device);
                observed_instances.insert(observation.key.device_instance);
                index.upsert(observation);
            }
            routers.extend(
                result
                    .routers
                    .into_iter()
                    .map(|router| router_observation(attachment_id, router)),
            );
            errors.extend(result.errors);
        }
        let devices = observed_instances
            .into_iter()
            .map(|instance| index.selection(instance))
            .collect();
        let index_revision = index.revision();
        drop(index);

        Ok(DiscoverySnapshot {
            generation,
            index_revision,
            devices,
            routers,
            errors,
        })
    }

    /// Walks B/IP BBMD tables and returns current router topology by attachment.
    pub async fn topology(
        &self,
        request: TopologyRequest,
    ) -> Result<RuntimeTopology, RuntimeError> {
        if request.max_bbmds_per_attachment == 0 {
            return Err(RuntimeError::invalid_config(
                "max_bbmds_per_attachment must be non-zero",
            ));
        }
        if let Some(seed) = request.bbmd_seeds.iter().find(|seed| seed.mac.len() != 6) {
            return Err(RuntimeError::invalid_attachment_config(
                seed.attachment_id,
                "B/IP BBMD MAC must contain exactly six bytes",
            ));
        }
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }

        let generation = self.inner.generation.load(Ordering::Acquire);
        let results = self.inner.registry.read().await.topology(&request).await;
        let mut attachments = Vec::with_capacity(results.len());
        let mut errors = Vec::new();
        for (attachment_id, result) in results {
            attachments.push(AttachmentTopology {
                attachment_id,
                local_mac: result.local_mac,
                bbmds: result.bbmds,
                routers: result.routers,
                truncated: result.truncated,
            });
            errors.extend(result.errors);
        }
        Ok(RuntimeTopology {
            generation,
            attachments,
            errors,
        })
    }

    /// Transactionally applies a newer complete attachment configuration.
    pub async fn reconcile(
        &self,
        revision: u64,
        desired: Vec<AttachmentConfig>,
    ) -> Result<ReconcileReport, RuntimeError> {
        validate_attachments(&desired)?;
        let _guard = self.inner.reconcile_lock.lock().await;
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }

        let current_revision = self.inner.config_revision.load(Ordering::Acquire);
        let (current_configs, current_epochs) = {
            let registry = self.inner.registry.read().await;
            (registry.configs(), registry.attachment_epochs())
        };
        let current_epochs = current_epochs
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        if revision < current_revision
            || (revision == current_revision && desired != current_configs)
        {
            return Err(RuntimeError::stale_revision(revision, current_revision));
        }
        if revision == current_revision {
            return Ok(ReconcileReport {
                revision,
                generation: self.inner.generation.load(Ordering::Acquire),
                added: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                idempotent: true,
            });
        }

        let registry_result = {
            let mut registry = self.inner.registry.write().await;
            registry.reconcile(&desired).await
        };
        let changes = match registry_result {
            Ok(changes) => changes,
            Err(error) => {
                // A transactional rollback may have recreated entries with new
                // transport epochs. Reconcile pumps to the registry that was
                // actually restored before returning the original failure.
                let _ = self.synchronize_i_am_pumps().await;
                let _ = self.synchronize_cov_pumps().await;
                return Err(error);
            }
        };
        {
            let mut device_index = self.inner.device_index.write().await;
            for id in &changes.removed {
                device_index.remove_attachment(*id);
                self.inner.capabilities.write().await.remove_attachment(*id);
                self.inner.values.write().await.remove_attachment(*id);
            }
            device_index.set_attachment_order(desired.iter().map(|config| config.id).collect());
            for config in &desired {
                device_index.set_attachment_health(config.id, true);
            }
        }
        for id in &changes.removed {
            self.drop_cov_attachment(*id).await;
        }
        let reconciled_epochs = self
            .inner
            .registry
            .read()
            .await
            .attachment_epochs()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        for id in &changes.updated {
            if current_epochs.get(id) != reconciled_epochs.get(id) {
                self.refresh_cov_attachment(*id).await;
            }
        }
        // Epoch synchronization preserves label-only pumps while replacing
        // pumps for concrete transport changes, additions, removals, and
        // rollback-restored entries.
        self.synchronize_cov_pumps().await?;
        self.synchronize_i_am_pumps().await?;
        self.inner
            .config_revision
            .store(revision, Ordering::Release);
        let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        for id in changes
            .added
            .iter()
            .chain(&changes.updated)
            .chain(&changes.removed)
        {
            self.inner
                .events
                .publish(generation, Some(*id), EventKind::AttachmentStateChanged)
                .await;
        }
        Ok(ReconcileReport {
            revision,
            generation,
            added: changes.added,
            updated: changes.updated,
            removed: changes.removed,
            idempotent: false,
        })
    }

    /// Atomically restores persisted direct or routed device paths.
    ///
    /// Every entry is validated against the current attachment transport before
    /// any index mutation occurs. This allows read/write/COV work immediately
    /// after process restart without a preparatory discovery broadcast.
    pub async fn restore_devices(
        &self,
        persisted: Vec<crate::PersistedDevice>,
    ) -> Result<crate::DeviceRestoreReport, RuntimeError> {
        use std::collections::BTreeSet;

        let _guard = self.inner.reconcile_lock.lock().await;
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }

        let configs = self.inner.registry.read().await.configs();
        let by_id = configs
            .iter()
            .map(|config| (config.id, &config.transport))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut keys = BTreeSet::new();
        for device in &persisted {
            if device.key.device_instance > 0x3f_ffff {
                return Err(RuntimeError::invalid_attachment_config(
                    device.key.attachment_id,
                    "device_instance must be in 0..=4194303",
                ));
            }
            if device.max_apdu_length == 0 {
                return Err(RuntimeError::invalid_attachment_config(
                    device.key.attachment_id,
                    "max_apdu_length must be non-zero",
                ));
            }
            if !keys.insert(device.key) {
                return Err(RuntimeError::invalid_attachment_config(
                    device.key.attachment_id,
                    format!(
                        "duplicate persisted device {} on the same attachment",
                        device.key.device_instance
                    ),
                ));
            }
            let transport = by_id
                .get(&device.key.attachment_id)
                .ok_or_else(|| RuntimeError::attachment_not_found(device.key.attachment_id))?;
            validate_persisted_path(device.key.attachment_id, transport, &device.path)?;
        }

        let mut report = crate::DeviceRestoreReport {
            added: 0,
            updated: 0,
            unchanged: 0,
            index_revision: 0,
        };
        let mut index = self.inner.device_index.write().await;
        for device in persisted {
            let change = index.upsert(crate::DeviceObservation {
                key: device.key,
                path: device.path,
                vendor_id: device.vendor_id,
                max_apdu_length: device.max_apdu_length,
                revision: 0,
            });
            match change.kind {
                crate::DeviceIndexChangeKind::Added => report.added += 1,
                crate::DeviceIndexChangeKind::Updated => report.updated += 1,
                crate::DeviceIndexChangeKind::Unchanged => report.unchanged += 1,
                crate::DeviceIndexChangeKind::Removed => unreachable!("upsert cannot remove"),
            }
        }
        report.index_revision = index.revision();
        Ok(report)
    }

    /// Stops new work, joins supervised tasks, and releases attachment ownership.
    pub async fn stop(&self) -> Result<StopReport, RuntimeError> {
        let _guard = self.inner.stop_lock.lock().await;
        let _reconcile_guard = self.inner.reconcile_lock.lock().await;
        let newly_stopped = !self.inner.stopped.swap(true, Ordering::AcqRel);
        let (tasks_joined, tasks_aborted) = if newly_stopped {
            self.inner
                .supervisor
                .stop(self.inner.shutdown_timeout)
                .await
        } else {
            (0, 0)
        };

        if newly_stopped {
            for sender in self.inner.cov_renewals.lock().await.values() {
                let _ = sender.send(true);
            }
            self.inner.cov_renewals.lock().await.clear();
            for control in self.inner.cov_pumps.lock().await.values() {
                let _ = control.cancel.send(true);
            }
            self.inner.cov_pumps.lock().await.clear();
            for control in self.inner.i_am_pumps.lock().await.values() {
                let _ = control.cancel.send(true);
            }
            self.inner.i_am_pumps.lock().await.clear();
            for sender in self.inner.inflight.lock().await.values() {
                let _ = sender.send(true);
            }
            self.inner.inflight.lock().await.clear();
            for item in self.inner.work_queue.lock().await.drain() {
                item.payload.fail(RuntimeError::stopped());
            }
            *self.inner.device_index.write().await = DeviceIndex::default();
            *self.inner.capabilities.write().await = CapabilityCache::default();
            *self.inner.values.write().await = ValueCache::default();
            *self.inner.observations.write().await = ObservationRegistry::default();
            self.inner.pending_observations.write().await.clear();
            self.inner.cov_last.lock().await.clear();
            self.inner
                .cov_notification_lag_count
                .store(0, Ordering::Release);
            self.inner
                .i_am_observation_lag_count
                .store(0, Ordering::Release);
            self.inner.events.clear().await;
            self.inner
                .events
                .publish(
                    self.inner.generation.load(Ordering::Acquire),
                    None,
                    EventKind::RuntimeStopped,
                )
                .await;
        }

        let attachment_stop = self.inner.registry.write().await.stop_all().await;
        let remaining_attachments = self.inner.registry.read().await.len();
        if let Err(error) = attachment_stop {
            return Err(error);
        }
        Ok(StopReport {
            newly_stopped,
            tasks_joined,
            tasks_aborted,
            remaining_attachments,
        })
    }
}

#[cfg(test)]
mod selection_health_tests {
    use super::synchronize_selection_health;
    use crate::{
        AttachmentHealth, AttachmentId, AttachmentState, DeviceIndex, DeviceKey, DeviceObservation,
        DevicePath,
    };

    fn observation(attachment_id: AttachmentId) -> DeviceObservation {
        DeviceObservation {
            key: DeviceKey {
                attachment_id,
                device_instance: 42,
            },
            path: DevicePath::Direct { mac: vec![1] },
            vendor_id: 1,
            max_apdu_length: 1476,
            revision: 0,
        }
    }

    fn health(id: AttachmentId, state: AttachmentState) -> AttachmentHealth {
        AttachmentHealth {
            id,
            label: "SC attachment".to_owned(),
            state,
            last_error: None,
            foreign_device_registration: None,
        }
    }

    #[test]
    fn failed_preferred_attachment_yields_to_healthy_alternative() {
        let failed_sc = AttachmentId::from(1);
        let healthy = AttachmentId::from(2);
        let mut index = DeviceIndex::new(vec![failed_sc, healthy]);
        index.upsert(observation(failed_sc));
        index.upsert(observation(healthy));
        index.set_preferred(42, Some(failed_sc));

        synchronize_selection_health(
            &mut index,
            &[
                health(failed_sc, AttachmentState::Failed),
                health(healthy, AttachmentState::Running),
            ],
        );

        let selection = index.selection(42);
        assert_eq!(selection.selected.unwrap().key.attachment_id, healthy);
        assert_eq!(selection.alternates[0].key.attachment_id, failed_sc);
    }
}
