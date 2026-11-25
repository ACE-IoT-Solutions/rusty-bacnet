use super::*;
use tokio::sync::oneshot;

#[cfg(test)]
pub(super) type StartBarrier = (
    oneshot::Sender<Arc<ManagedAttachment>>,
    oneshot::Receiver<()>,
);

// A startup worker can unwind after binding an attachment but before offering
// a result. The durable owner map then still has to be drained on its executor.
struct Unpublished {
    runtime: BacnetRuntime,
    executor: tokio::runtime::Handle,
    armed: bool,
}

impl Drop for Unpublished {
    fn drop(&mut self) {
        if self.armed {
            let runtime = self.runtime.clone();
            self.executor
                .spawn(async move { runtime.drain_unclaimed().await });
        }
    }
}

impl BacnetRuntime {
    /// Applies a newer complete attachment configuration transactionally.
    ///
    /// After admission the transition runs to completion even if the caller drops
    /// its future. A failed replacement restores the previous configuration when
    /// cleanup succeeds. Unresolved shutdown or rollback closes command admission
    /// and retains any outstanding owners for [`Self::stop`].
    pub async fn reconcile(
        &self,
        revision: u64,
        desired: Vec<AttachmentConfig>,
    ) -> Result<ReconcileReport, RuntimeError> {
        let desired = resolve_interfaces(desired).await?;
        validate_attachments(&desired)?;
        let permit = self
            .inner
            .lifecycle
            .clone()
            .acquire_owned()
            .await
            .expect("lifecycle admission stays open");
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        let active_operations = *self.inner.operation_count.borrow();
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
        let current_configs: BTreeMap<_, _> = self
            .inner
            .attachments
            .lock()
            .unwrap()
            .iter()
            .map(|(id, attachment)| (*id, attachment.config.clone()))
            .collect();
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

        // Once admitted this transaction owns every remaining step, including compensation.
        self.inner.stopped.store(true, Ordering::Release);
        let runtime = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            runtime
                .replace_set(desired, current_configs.into_values().collect())
                .await?;
            runtime.inner.revision.store(revision, Ordering::Release);
            let generation = runtime.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
            runtime.inner.stopped.store(false, Ordering::Release);
            Ok(ReconcileReport {
                revision,
                generation,
                added,
                updated,
                removed,
                idempotent: false,
            })
        })
        .await
        .map_err(|error| RuntimeError::internal(format!("reconciliation task ended: {error}")))?
    }
}

impl BacnetRuntime {
    pub(super) async fn initialize(self) -> Result<Self, RuntimeError> {
        let (offered, ready) = oneshot::channel();
        let (claim, claimed) = oneshot::channel();
        let mut guard = Unpublished {
            runtime: self.clone(),
            executor: tokio::runtime::Handle::current(),
            armed: true,
        };
        tokio::spawn(async move {
            // Keep the whole guard in this task even before its first poll.
            let guard = &mut guard;
            let runtime = &guard.runtime;
            let configs = &runtime.inner.config.attachments;
            let mut result = Ok(());
            for config in configs {
                if offered.is_closed() {
                    break;
                }
                if let Err(error) = runtime.start_one(config).await {
                    result = Err(error);
                    break;
                }
            }
            if let Err(error) = result {
                let cleanup = runtime
                    .drain_attachments(Some(runtime.shutdown_deadline()))
                    .await;
                let failed = cleanup.is_err();
                let error = match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => cleanup_error(error, "startup cleanup", cleanup),
                };
                let _ = offered.send(Err(error));
                if failed {
                    runtime.drain_unclaimed().await;
                }
            } else {
                let delivered = offered.send(Ok(())).is_ok();
                #[cfg(test)]
                if let Some(marker) = runtime.inner.offered.lock().unwrap().take() {
                    let _ = marker.send(());
                }
                if !delivered || claimed.await.is_err() {
                    runtime.drain_unclaimed().await;
                }
            }
            guard.armed = false;
        });
        // Claim synchronously in the poll that hands the runtime to its caller.
        // Cancellation after the offer but before this poll drops the claim sender.
        ready.await.map_err(|_| {
            RuntimeError::internal("startup task ended before offering its result")
        })??;
        claim.send(()).map_err(|_| {
            RuntimeError::internal("startup task ended before accepting its result")
        })?;
        Ok(self)
    }

    fn shutdown_deadline(&self) -> Instant {
        Instant::now() + self.inner.config.shutdown_timeout
    }

    // No caller owns an unpublished runtime. Remain responsible for its joins even
    // when a bounded startup cleanup has already returned an error to its caller.
    async fn drain_unclaimed(&self) {
        while self.drain_attachments(None).await.is_err() {
            tokio::time::sleep(self.inner.config.shutdown_timeout).await;
        }
        #[cfg(test)]
        if let Some(done) = self.inner.cleaned.lock().unwrap().take() {
            let _ = done.send(());
        }
    }

    async fn start_one(&self, config: &AttachmentConfig) -> Result<(), RuntimeError> {
        let owner = Arc::new(start_attachment(config.clone()).await?);
        self.inner
            .attachments
            .lock()
            .unwrap()
            .insert(config.id, owner.clone());
        #[cfg(test)]
        {
            let hook = self.inner.started.lock().unwrap().remove(&config.id);
            if let Some((entered, resume)) = hook {
                let _ = entered.send(owner);
                resume.await.expect("injected startup worker loss");
            }
        }
        Ok(())
    }

    async fn start_set(&self, configs: &[AttachmentConfig]) -> Result<(), RuntimeError> {
        for config in configs {
            self.start_one(config).await?;
        }
        Ok(())
    }

    pub(super) async fn drain_attachments(
        &self,
        deadline: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        let owners = self.inner.attachments.lock().unwrap().clone();
        let mut failure = None;
        for (id, owner) in owners {
            match owner.stop(deadline).await {
                Ok(()) => {
                    self.inner.attachments.lock().unwrap().remove(&id);
                }
                Err(error) => {
                    failure = Some(match failure {
                        None => error,
                        Some(earlier) => RuntimeError {
                            message: format!(
                                "{}; attachment {id} shutdown also failed ({error})",
                                earlier
                            ),
                            ..earlier
                        },
                    });
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn replace_set(
        &self,
        desired: Vec<AttachmentConfig>,
        previous: Vec<AttachmentConfig>,
    ) -> Result<(), RuntimeError> {
        self.drain_attachments(Some(self.shutdown_deadline()))
            .await?;
        let Err(original) = self.start_set(&desired).await else {
            return Ok(());
        };
        if let Err(cleanup) = self.drain_attachments(Some(self.shutdown_deadline())).await {
            return Err(cleanup_error(original, "replacement cleanup", cleanup));
        }
        match self.start_set(&previous).await {
            Ok(()) => {
                self.inner.stopped.store(false, Ordering::Release);
                Err(original)
            }
            Err(rollback) => {
                let error = cleanup_error(original, "rollback", rollback);
                match self.drain_attachments(Some(self.shutdown_deadline())).await {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(cleanup_error(error, "rollback cleanup", cleanup)),
                }
            }
        }
    }
}

fn cleanup_error(original: RuntimeError, stage: &str, failure: RuntimeError) -> RuntimeError {
    RuntimeError {
        code: crate::ErrorCode::RollbackFailed,
        retryable: false,
        attachment_id: failure.attachment_id,
        message: format!("{original}; {stage} failed ({failure})"),
    }
}
