mod transition;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::time::Instant;

use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::AbortHandle;

use crate::attachment::{ManagedAttachment, start_attachment};
use crate::batch::{
    DevicePath, OperationId, ReadBatch, ReadBatchOperation, ReadOutcome, WriteBatch,
    WriteBatchOperation, WriteOutcome,
};
use crate::config::{
    AttachmentConfig, AttachmentId, RuntimeConfig, resolve_interfaces, validate_attachments,
};
use crate::error::RuntimeError;
use crate::health::{ReconcileReport, RuntimeHealth, StopReport};

struct RuntimeInner {
    generation: AtomicU64,
    revision: AtomicU64,
    stopped: AtomicBool,
    next_operation: AtomicU64,
    config: RuntimeConfig,
    attachments: StdMutex<BTreeMap<AttachmentId, Arc<ManagedAttachment>>>,
    operation_slots: Arc<Semaphore>,
    operations: StdMutex<BTreeMap<OperationId, Option<AbortHandle>>>,
    operation_count: watch::Sender<usize>,
    lifecycle: Arc<Semaphore>,
    #[cfg(test)]
    started: StdMutex<BTreeMap<AttachmentId, transition::StartBarrier>>,
    #[cfg(test)]
    offered: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
    #[cfg(test)]
    cleaned: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
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
        let mut operations = self
            .inner
            .operations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        operations.remove(&self.id);
        self.inner.operation_count.send_replace(operations.len());
    }
}

/// Bounded supervised BACnet/IP runtime.
#[derive(Clone)]
pub struct BacnetRuntime {
    inner: Arc<RuntimeInner>,
}

impl BacnetRuntime {
    /// Validates configuration and starts all configured B/IP attachments.
    pub async fn start(mut config: RuntimeConfig) -> Result<Self, RuntimeError> {
        config.attachments = resolve_interfaces(config.attachments).await?;
        config.validate()?;
        Self::unstarted(config).initialize().await
    }

    fn unstarted(config: RuntimeConfig) -> Self {
        let (operation_count, _) = watch::channel(0);
        let inner = Arc::new(RuntimeInner {
            generation: AtomicU64::new(1),
            revision: AtomicU64::new(1),
            stopped: AtomicBool::new(false),
            next_operation: AtomicU64::new(1),
            operation_slots: Arc::new(Semaphore::new(
                config
                    .max_inflight_operations
                    .min(config.scheduler_capacity),
            )),
            operations: StdMutex::new(BTreeMap::new()),
            operation_count,
            attachments: StdMutex::new(BTreeMap::new()),
            lifecycle: Arc::new(Semaphore::new(1)),
            #[cfg(test)]
            started: StdMutex::default(),
            #[cfg(test)]
            offered: StdMutex::default(),
            #[cfg(test)]
            cleaned: StdMutex::default(),
            config,
        });
        Self { inner }
    }

    /// Returns a point-in-time runtime health snapshot without BACnet I/O.
    pub async fn health(&self) -> RuntimeHealth {
        let _lifecycle = self
            .inner
            .lifecycle
            .acquire()
            .await
            .expect("lifecycle admission stays open");
        let attachments = self.inner.attachments.lock().unwrap().clone();
        let mut health = Vec::with_capacity(attachments.len());
        for attachment in attachments.values() {
            health.push(attachment.health().await);
        }
        let attachment_task_count = attachments.len();
        let operation_count = *self.inner.operation_count.borrow();
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

    /// Closes admission, cancels batches and joins attachment shutdowns.
    ///
    /// After cancellation or timeout, call again to resume the outstanding joins.
    pub async fn stop(&self) -> Result<StopReport, RuntimeError> {
        let _lifecycle = self
            .inner
            .lifecycle
            .acquire()
            .await
            .expect("lifecycle admission stays open");
        let newly_stopped = !self.inner.stopped.swap(true, Ordering::AcqRel);
        let has_attachments = !self.inner.attachments.lock().unwrap().is_empty();
        let has_operations = *self.inner.operation_count.borrow() != 0;
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
        let operations_stopped =
            match tokio::time::timeout_at(deadline, operation_counts.wait_for(|count| *count == 0))
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
        let count = self.inner.attachments.lock().unwrap().len();
        let attachments_stopped = self.drain_attachments(Some(deadline)).await;
        match (operations_stopped, attachments_stopped) {
            (Err(drain), Err(cleanup)) => {
                return Err(RuntimeError {
                    message: format!("{drain}; attachment cleanup also failed ({cleanup})"),
                    ..drain
                });
            }
            (Err(error), _) | (_, Err(error)) => return Err(error),
            (Ok(()), Ok(())) => {}
        }
        Ok(StopReport {
            newly_stopped,
            tasks_joined: count,
            tasks_aborted: aborted,
            remaining_attachments: 0,
        })
    }

    /// Schedules one ordered read batch, rejecting when all operation slots are busy.
    pub async fn read_batch(&self, batch: ReadBatch) -> Result<ReadBatchOperation, RuntimeError> {
        let _lifecycle = self
            .inner
            .lifecycle
            .acquire()
            .await
            .expect("lifecycle admission stays open");
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
        let _lifecycle = self
            .inner
            .lifecycle
            .acquire()
            .await
            .expect("lifecycle admission stays open");
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
        let mut operations = self
            .inner
            .operations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        operations.insert(id, None);
        self.inner.operation_count.send_replace(operations.len());
        Ok(OperationGuard::new(id, inner, permit))
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
                let outcome = match self.attachment(item.attachment_id) {
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
                let outcome = match self.attachment(item.attachment_id) {
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

    fn attachment(&self, id: AttachmentId) -> Result<Arc<ManagedAttachment>, RuntimeError> {
        self.attachments
            .lock()
            .unwrap()
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
