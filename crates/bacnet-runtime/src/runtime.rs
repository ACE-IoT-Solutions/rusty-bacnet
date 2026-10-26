use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::AbortHandle;

use crate::attachment::{start_attachment, ManagedAttachment};
use crate::batch::{
    DevicePath, OperationId, ReadBatch, ReadBatchOperation, ReadOutcome, WriteBatch,
    WriteBatchOperation, WriteOutcome,
};
use crate::config::{validate_attachments, AttachmentConfig, AttachmentId, RuntimeConfig};
use crate::error::RuntimeError;
use crate::health::{AttachmentState, ReconcileReport, RuntimeHealth, StopReport};

struct RuntimeInner {
    generation: AtomicU64,
    revision: AtomicU64,
    stopped: AtomicBool,
    next_operation: AtomicU64,
    active_operations: AtomicUsize,
    config: RuntimeConfig,
    attachments: Mutex<BTreeMap<AttachmentId, Arc<ManagedAttachment>>>,
    operation_slots: Arc<Semaphore>,
    operations: StdMutex<BTreeMap<OperationId, Option<AbortHandle>>>,
    operation_count: watch::Sender<usize>,
    lifecycle: Mutex<()>,
}

struct OperationGuard {
    id: OperationId,
    inner: Arc<RuntimeInner>,
    _permit: OwnedSemaphorePermit,
}

impl OperationGuard {
    fn new(id: OperationId, inner: Arc<RuntimeInner>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            id,
            inner,
            _permit: permit,
        }
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.inner
            .operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.id);
        let remaining = self.inner.active_operations.fetch_sub(1, Ordering::AcqRel) - 1;
        self.inner.operation_count.send_replace(remaining);
    }
}

/// Bounded supervised BACnet/IP runtime.
#[derive(Clone)]
pub struct BacnetRuntime {
    inner: Arc<RuntimeInner>,
}

impl BacnetRuntime {
    /// Validates configuration and starts all configured B/IP attachments.
    pub async fn start(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        config.validate()?;
        let (operation_count, _) = watch::channel(0);
        let inner = Arc::new(RuntimeInner {
            generation: AtomicU64::new(1),
            revision: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            next_operation: AtomicU64::new(1),
            active_operations: AtomicUsize::new(0),
            operation_slots: Arc::new(Semaphore::new(
                config
                    .max_inflight_operations
                    .min(config.scheduler_capacity),
            )),
            operations: StdMutex::new(BTreeMap::new()),
            operation_count,
            attachments: Mutex::new(BTreeMap::new()),
            lifecycle: Mutex::new(()),
            config: config.clone(),
        });
        let runtime = Self { inner };
        let started = Self::start_map(&config.attachments).await;
        match started {
            Ok(attachments) => {
                *runtime.inner.attachments.lock().await = attachments;
                Ok(runtime)
            }
            Err((error, attachments)) => {
                let deadline = Instant::now() + config.shutdown_timeout;
                let _ = Self::stop_map(attachments, deadline).await;
                Err(error)
            }
        }
    }

    /// Returns a point-in-time runtime health snapshot without BACnet I/O.
    pub async fn health(&self) -> RuntimeHealth {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let attachments = self.inner.attachments.lock().await;
        let mut health = Vec::with_capacity(attachments.len());
        for attachment in attachments.values() {
            health.push(attachment.health().await);
        }
        let attachment_task_count = health
            .iter()
            .filter(|item| {
                matches!(
                    item.state,
                    AttachmentState::Starting
                        | AttachmentState::Running
                        | AttachmentState::Stopping
                )
            })
            .count();
        let operation_count = self.inner.active_operations.load(Ordering::Acquire);
        RuntimeHealth {
            generation: self.inner.generation.load(Ordering::Acquire),
            accepting_commands: !self.inner.stopped.load(Ordering::Acquire),
            task_count: attachment_task_count + operation_count,
            supervisor_task_count: 0,
            attachment_task_count,
            operation_count,
            event_queue_depth: 0,
            event_lag_count: 0,
            cov_notification_lag_count: 0,
            i_am_observation_lag_count: 0,
            device_count: 0,
            device_observation_count: 0,
            capability_count: 0,
            cached_value_count: 0,
            observation_count: 0,
            attachments: health,
        }
    }

    /// Stops all attachments. Repeated calls are idempotent.
    pub async fn stop(&self) -> Result<StopReport, RuntimeError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let newly_stopped = !self.inner.stopped.swap(true, Ordering::AcqRel);
        let has_attachments = !self.inner.attachments.lock().await.is_empty();
        let has_operations = self.inner.active_operations.load(Ordering::Acquire) != 0;
        if !newly_stopped && !has_attachments && !has_operations {
            return Ok(StopReport {
                newly_stopped: false,
                tasks_joined: 0,
                tasks_aborted: 0,
                remaining_attachments: 0,
            });
        }
        let deadline = Instant::now() + self.inner.config.shutdown_timeout;
        let aborted = self.inner.abort_operations();
        let mut operation_counts = self.inner.operation_count.subscribe();
        let operations_stopped = match tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            operation_counts.wait_for(|count| *count == 0),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(RuntimeError::internal(format!(
                "operation counter closed: {error}"
            ))),
            Err(_) => Err(RuntimeError {
                code: crate::ErrorCode::DeadlineExceeded,
                retryable: true,
                attachment_id: None,
                message: "runtime operation shutdown timed out".to_owned(),
            }),
        };
        let attachments = self.inner.attachments.lock().await.clone();
        let count = attachments.len();
        let attachment_result = Self::stop_map(attachments, deadline).await;
        if attachment_result.is_ok() {
            self.inner.attachments.lock().await.clear();
        }
        attachment_result?;
        operations_stopped?;
        Ok(StopReport {
            newly_stopped,
            tasks_joined: count,
            tasks_aborted: aborted,
            remaining_attachments: 0,
        })
    }

    /// Applies a newer complete attachment configuration transactionally.
    ///
    /// Reconciliation intentionally stops and rebinds the complete set. This
    /// keeps rollback and bind-conflict behavior deterministic while the
    /// runtime is still B/IP-only; a future slice can add per-attachment
    /// rolling replacement once its task ownership is specified.
    pub async fn reconcile(
        &self,
        revision: u64,
        desired: Vec<AttachmentConfig>,
    ) -> Result<ReconcileReport, RuntimeError> {
        validate_attachments(&desired)?;
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        let active_operations = self.inner.active_operations.load(Ordering::Acquire);
        if active_operations != 0 {
            return Err(RuntimeError {
                code: crate::ErrorCode::Backpressure,
                retryable: true,
                attachment_id: None,
                message: format!(
                    "cannot reconcile while {active_operations} batch operation(s) are active"
                ),
            });
        }
        let current_revision = self.inner.revision.load(Ordering::Acquire);
        let current = self.inner.attachments.lock().await;
        let current_configs: BTreeMap<_, _> = current
            .iter()
            .map(|(id, attachment)| (*id, attachment.config.clone()))
            .collect();
        drop(current);
        let desired_configs: BTreeMap<_, _> = desired
            .iter()
            .map(|attachment| (attachment.id, attachment.clone()))
            .collect();
        if revision < current_revision {
            return Err(RuntimeError {
                code: crate::ErrorCode::StaleRevision,
                retryable: false,
                attachment_id: None,
                message: format!("reconcile revision {revision} is older than {current_revision}"),
            });
        }
        if revision == current_revision && current_configs == desired_configs {
            return Ok(ReconcileReport {
                revision,
                generation: self.inner.generation.load(Ordering::Acquire),
                added: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                idempotent: true,
            });
        }
        if revision == current_revision {
            return Err(RuntimeError {
                code: crate::ErrorCode::StaleRevision,
                retryable: false,
                attachment_id: None,
                message: format!("reconcile revision {revision} has already been applied"),
            });
        }

        let added = desired_configs
            .keys()
            .filter(|id| !current_configs.contains_key(id))
            .copied()
            .collect();
        let removed = current_configs
            .keys()
            .filter(|id| !desired_configs.contains_key(id))
            .copied()
            .collect();
        let updated = desired_configs
            .keys()
            .filter(|id| {
                current_configs
                    .get(id)
                    .is_some_and(|current| Some(current) != desired_configs.get(id))
            })
            .copied()
            .collect();

        let old = self.inner.attachments.lock().await.clone();
        let deadline = Instant::now() + self.inner.config.shutdown_timeout;
        if let Err(error) = Self::stop_map(old, deadline).await {
            return self
                .restore_after_failed_reconcile(&current_configs, error)
                .await;
        }
        match Self::start_map(&desired).await {
            Ok(new_attachments) => {
                *self.inner.attachments.lock().await = new_attachments;
                self.inner.revision.store(revision, Ordering::Release);
                let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
                Ok(ReconcileReport {
                    revision,
                    generation,
                    added,
                    updated,
                    removed,
                    idempotent: false,
                })
            }
            Err((error, partial)) => {
                let deadline = Instant::now() + self.inner.config.shutdown_timeout;
                let _ = Self::stop_map(partial, deadline).await;
                self.restore_after_failed_reconcile(&current_configs, error)
                    .await
            }
        }
    }

    async fn restore_after_failed_reconcile(
        &self,
        current: &BTreeMap<AttachmentId, AttachmentConfig>,
        original: RuntimeError,
    ) -> Result<ReconcileReport, RuntimeError> {
        match Self::start_map(&current.values().cloned().collect::<Vec<_>>()).await {
            Ok(restored) => {
                *self.inner.attachments.lock().await = restored;
                Err(original)
            }
            Err((rollback, partial_restore)) => {
                let deadline = Instant::now() + self.inner.config.shutdown_timeout;
                let _ = Self::stop_map(partial_restore, deadline).await;
                self.inner.stopped.store(true, Ordering::Release);
                Err(RuntimeError {
                    code: crate::ErrorCode::RollbackFailed,
                    retryable: false,
                    attachment_id: rollback.attachment_id,
                    message: format!("reconcile failed ({original}); rollback failed ({rollback})"),
                })
            }
        }
    }

    /// Schedules one ordered read batch, rejecting when all operation slots are busy.
    pub async fn read_batch(&self, batch: ReadBatch) -> Result<ReadBatchOperation, RuntimeError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.validate_batch(batch.items.len())?;
        validate_read_items(&batch)?;
        let id = OperationId(self.inner.next_operation.fetch_add(1, Ordering::Relaxed));
        let inner = Arc::clone(&self.inner);
        let guard = self.reserve_operation(id, Arc::clone(&inner))?;
        let task = tokio::spawn(async move {
            let _guard = guard;
            inner.execute_read(batch).await
        });
        self.inner.register_abort_handle(id, task.abort_handle());
        Ok(ReadBatchOperation::new(id, task))
    }

    /// Schedules one ordered write batch, rejecting when all operation slots are busy.
    pub async fn write_batch(
        &self,
        batch: WriteBatch,
    ) -> Result<WriteBatchOperation, RuntimeError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.validate_batch(batch.items.len())?;
        validate_write_items(&batch)?;
        let id = OperationId(self.inner.next_operation.fetch_add(1, Ordering::Relaxed));
        let inner = Arc::clone(&self.inner);
        let guard = self.reserve_operation(id, Arc::clone(&inner))?;
        let task = tokio::spawn(async move {
            let _guard = guard;
            inner.execute_write(batch).await
        });
        self.inner.register_abort_handle(id, task.abort_handle());
        Ok(WriteBatchOperation::new(id, task))
    }

    fn validate_batch(&self, length: usize) -> Result<(), RuntimeError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        if length > self.inner.config.max_batch_items {
            return Err(RuntimeError::invalid_config(format!(
                "batch has {length} items; maximum is {}",
                self.inner.config.max_batch_items
            )));
        }
        Ok(())
    }

    fn reserve_operation(
        &self,
        id: OperationId,
        inner: Arc<RuntimeInner>,
    ) -> Result<OperationGuard, RuntimeError> {
        let permit = self
            .inner
            .operation_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError {
                code: crate::ErrorCode::Backpressure,
                retryable: true,
                attachment_id: None,
                message: "runtime operation capacity is exhausted".to_owned(),
            })?;
        let active = self.inner.active_operations.fetch_add(1, Ordering::AcqRel) + 1;
        self.inner.operation_count.send_replace(active);
        self.inner
            .operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, None);
        Ok(OperationGuard::new(id, inner, permit))
    }

    async fn start_map(
        configs: &[AttachmentConfig],
    ) -> Result<
        BTreeMap<AttachmentId, Arc<ManagedAttachment>>,
        (RuntimeError, BTreeMap<AttachmentId, Arc<ManagedAttachment>>),
    > {
        let mut started = BTreeMap::new();
        for config in configs {
            match start_attachment(config.clone()).await {
                Ok(attachment) => {
                    started.insert(config.id, Arc::new(attachment));
                }
                Err(error) => return Err((error, started)),
            }
        }
        Ok(started)
    }

    async fn stop_map(
        attachments: BTreeMap<AttachmentId, Arc<ManagedAttachment>>,
        deadline: Instant,
    ) -> Result<(), RuntimeError> {
        let mut first_error = None;
        for attachment in attachments.into_values() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Err(error) = attachment.stop(remaining).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl RuntimeInner {
    fn register_abort_handle(&self, id: OperationId, handle: AbortHandle) {
        if let Some(slot) = self
            .operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&id)
        {
            *slot = Some(handle);
        }
    }

    fn abort_operations(&self) -> usize {
        let operations = self
            .operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut aborted = 0;
        for handle in operations.values().flatten() {
            handle.abort();
            aborted += 1;
        }
        aborted
    }

    async fn execute_read(
        self: &Arc<Self>,
        batch: ReadBatch,
    ) -> Result<Vec<ReadOutcome>, RuntimeError> {
        tokio::time::timeout(batch.timeout, async {
            let mut outcomes = Vec::with_capacity(batch.items.len());
            for item in batch.items {
                if self.stopped.load(Ordering::Acquire) {
                    return Err(RuntimeError::stopped());
                }
                let outcome = match self.attachment(item.attachment_id).await {
                    Ok(attachment) => match attachment.read_property(&item.target).await {
                        Ok(raw_value) => ReadOutcome {
                            input_index: item.input_index,
                            attachment_id: item.attachment_id,
                            path: Some(item.target.path),
                            raw_value: Some(raw_value),
                            error: None,
                        },
                        Err(error) => ReadOutcome {
                            input_index: item.input_index,
                            attachment_id: item.attachment_id,
                            path: Some(item.target.path),
                            raw_value: None,
                            error: Some(error),
                        },
                    },
                    Err(error) => ReadOutcome {
                        input_index: item.input_index,
                        attachment_id: item.attachment_id,
                        path: None,
                        raw_value: None,
                        error: Some(error),
                    },
                };
                outcomes.push(outcome);
            }
            Ok(outcomes)
        })
        .await
        .map_err(|_| RuntimeError::deadline())?
    }

    async fn execute_write(
        self: &Arc<Self>,
        batch: WriteBatch,
    ) -> Result<Vec<WriteOutcome>, RuntimeError> {
        tokio::time::timeout(batch.timeout, async {
            let mut outcomes = Vec::with_capacity(batch.items.len());
            for item in batch.items {
                if self.stopped.load(Ordering::Acquire) {
                    return Err(RuntimeError::stopped());
                }
                let outcome = match self.attachment(item.attachment_id).await {
                    Ok(attachment) => match attachment
                        .write_property(&item.target, item.raw_value, item.priority)
                        .await
                    {
                        Ok(()) => WriteOutcome {
                            input_index: item.input_index,
                            attachment_id: item.attachment_id,
                            path: Some(item.target.path),
                            written: true,
                            error: None,
                        },
                        Err(error) => WriteOutcome {
                            input_index: item.input_index,
                            attachment_id: item.attachment_id,
                            path: Some(item.target.path),
                            written: false,
                            error: Some(error),
                        },
                    },
                    Err(error) => WriteOutcome {
                        input_index: item.input_index,
                        attachment_id: item.attachment_id,
                        path: None,
                        written: false,
                        error: Some(error),
                    },
                };
                outcomes.push(outcome);
            }
            Ok(outcomes)
        })
        .await
        .map_err(|_| RuntimeError::deadline())?
    }

    async fn attachment(&self, id: AttachmentId) -> Result<Arc<ManagedAttachment>, RuntimeError> {
        self.attachments
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| RuntimeError::attachment_not_found(id))
    }
}

fn validate_read_items(batch: &ReadBatch) -> Result<(), RuntimeError> {
    for item in &batch.items {
        validate_target(item.attachment_id, &item.target.path)?;
    }
    Ok(())
}

fn validate_write_items(batch: &WriteBatch) -> Result<(), RuntimeError> {
    for item in &batch.items {
        validate_target(item.attachment_id, &item.target.path)?;
        if item
            .priority
            .is_some_and(|priority| !(1..=16).contains(&priority))
        {
            return Err(RuntimeError::invalid_config(
                "BACnet write priority must be in 1..=16",
            ));
        }
    }
    Ok(())
}

fn validate_target(id: AttachmentId, path: &DevicePath) -> Result<(), RuntimeError> {
    match path {
        DevicePath::Direct { mac } if mac.len() != 6 => Err(RuntimeError::attachment_config(
            id,
            "direct B/IP MAC must contain six octets",
        )),
        DevicePath::Routed { ingress_mac, .. } if ingress_mac.len() != 6 => Err(
            RuntimeError::attachment_config(id, "B/IP router MAC must contain six octets"),
        ),
        DevicePath::Routed { dnet, .. } if !(1..=u16::MAX - 1).contains(dnet) => Err(
            RuntimeError::attachment_config(id, "routed DNET must be in 1..=65534"),
        ),
        DevicePath::Routed { dadr, .. }
            if dadr.is_empty()
                || dadr.len() > bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN =>
        {
            Err(RuntimeError::attachment_config(
                id,
                format!(
                    "routed DADR must contain 1..={} octets",
                    bacnet_types::constructed::BACnetAddress::MAX_MAC_LEN
                ),
            ))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
