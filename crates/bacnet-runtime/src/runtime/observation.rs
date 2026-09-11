use super::*;

impl BacnetRuntime {
    /// Transactionally applies a newer complete managed-COV observation plan.
    pub async fn apply_observation_plan(
        &self,
        plan: ObservationPlan,
    ) -> Result<ObservationReport, RuntimeError> {
        let _guard = self.inner.observation_lock.lock().await;
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(RuntimeError::stopped());
        }
        let changes = self.inner.observations.read().await.diff(plan)?;
        if changes.idempotent {
            return Ok(self.inner.observations.write().await.commit(changes));
        }
        let old_updates = {
            let observations = self.inner.observations.read().await;
            changes
                .updated
                .iter()
                .filter_map(|spec| observations.get(spec.key).cloned())
                .collect::<Vec<_>>()
        };
        let pump_attachments: std::collections::BTreeSet<_> = changes
            .added
            .iter()
            .chain(&changes.updated)
            .map(|spec| spec.key.device.attachment_id)
            .collect();
        for attachment_id in pump_attachments {
            self.ensure_cov_pump(attachment_id).await?;
        }
        *self.inner.pending_observations.write().await = changes.desired.clone();
        let mut rollback = Vec::new();
        for spec in &changes.added {
            if let Err(error) = self.set_cov_subscription(spec, true).await {
                self.rollback_cov(rollback).await;
                self.inner.pending_observations.write().await.clear();
                return Err(error);
            }
            rollback.push(CovRollback::Unsubscribe(spec.clone()));
        }
        for (new, old) in changes.updated.iter().zip(old_updates) {
            if let Err(error) = self.set_cov_subscription(new, true).await {
                self.rollback_cov(rollback).await;
                self.inner.pending_observations.write().await.clear();
                return Err(error);
            }
            rollback.push(CovRollback::Subscribe(old));
        }
        for spec in &changes.removed {
            if let Err(error) = self.set_cov_subscription(spec, false).await {
                self.rollback_cov(rollback).await;
                self.inner.pending_observations.write().await.clear();
                return Err(error);
            }
            rollback.push(CovRollback::Subscribe(spec.clone()));
        }
        let added = changes.added.clone();
        let updated = changes.updated.clone();
        let removed = changes.removed.clone();
        let report = self.inner.observations.write().await.commit(changes);
        self.inner.pending_observations.write().await.clear();
        for spec in removed.iter().chain(&updated) {
            self.stop_cov_renewal(spec.key).await;
            self.inner.cov_last.lock().await.remove(&spec.key);
        }
        for spec in added.iter().chain(&updated) {
            self.start_cov_renewal(spec.clone()).await;
        }
        self.synchronize_cov_pumps().await?;
        Ok(report)
    }

    pub(super) async fn rollback_cov(&self, rollback: Vec<CovRollback>) {
        for action in rollback.into_iter().rev() {
            let (spec, subscribe) = match action {
                CovRollback::Subscribe(spec) => (spec, true),
                CovRollback::Unsubscribe(spec) => (spec, false),
            };
            let _ = self.set_cov_subscription(&spec, subscribe).await;
        }
    }

    pub(super) async fn set_cov_subscription(
        &self,
        spec: &ObservationSpec,
        subscribe: bool,
    ) -> Result<(), RuntimeError> {
        let observation = self
            .inner
            .device_index
            .read()
            .await
            .get(spec.key.device)
            .cloned()
            .ok_or_else(|| RuntimeError::device_not_found(spec.key.device))?;
        let registry = self.inner.registry.read().await;
        let transport = registry.transport(spec.key.device.attachment_id)?;
        if subscribe {
            transport
                .subscribe_cov(spec.key.device.attachment_id, &observation.path, spec)
                .await
        } else {
            transport
                .unsubscribe_cov(spec.key.device.attachment_id, &observation.path, spec)
                .await
        }
    }

    pub(super) async fn start_cov_renewal(&self, spec: ObservationSpec) {
        self.stop_cov_renewal(spec.key).await;
        let (cancel, mut cancelled) = watch::channel(false);
        self.inner
            .cov_renewals
            .lock()
            .await
            .insert(spec.key, cancel);
        let weak = Arc::downgrade(&self.inner);
        let delay = Duration::from_secs(u64::from(spec.lifetime_seconds)) - spec.renewal_margin;
        let _ = self
            .inner
            .supervisor
            .spawn(move |mut global_stop| async move {
                loop {
                    tokio::select! {
                        _ = global_stop.changed() => return,
                        _ = cancelled.changed() => return,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    let runtime = BacnetRuntime { inner };
                    match runtime.set_cov_subscription(&spec, true).await {
                        Ok(()) => {
                            runtime
                                .inner
                                .events
                                .publish(
                                    runtime.inner.generation.load(Ordering::Acquire),
                                    Some(spec.key.device.attachment_id),
                                    EventKind::CovRenewed { key: spec.key },
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .inner
                                .events
                                .publish(
                                    runtime.inner.generation.load(Ordering::Acquire),
                                    Some(spec.key.device.attachment_id),
                                    EventKind::CovRenewalFailed {
                                        key: spec.key,
                                        code: error.code,
                                    },
                                )
                                .await;
                            tokio::select! {
                                _ = global_stop.changed() => return,
                                _ = cancelled.changed() => return,
                                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                            }
                        }
                    }
                }
            })
            .await;
    }

    pub(super) async fn stop_cov_renewal(&self, key: ObservationKey) {
        if let Some(cancel) = self.inner.cov_renewals.lock().await.remove(&key) {
            let _ = cancel.send(true);
        }
    }

    pub(super) async fn ensure_cov_pump(
        &self,
        attachment_id: crate::AttachmentId,
    ) -> Result<(), RuntimeError> {
        let _lifecycle = self.inner.cov_pump_lifecycle.lock().await;
        self.ensure_cov_pump_locked(attachment_id).await
    }

    pub(super) async fn ensure_cov_pump_locked(
        &self,
        attachment_id: crate::AttachmentId,
    ) -> Result<(), RuntimeError> {
        let registry_epoch = self
            .inner
            .registry
            .read()
            .await
            .attachment_epoch(attachment_id)
            .ok_or_else(|| RuntimeError::attachment_not_found(attachment_id))?;
        {
            let mut pumps = self.inner.cov_pumps.lock().await;
            if pumps
                .get(&attachment_id)
                .is_some_and(|control| control.attachment_epoch == registry_epoch)
            {
                return Ok(());
            }
            if let Some(stale) = pumps.remove(&attachment_id) {
                let _ = stale.cancel.send(true);
            }
        }
        let (attachment_epoch, mut receiver) = self
            .inner
            .registry
            .read()
            .await
            .cov_receiver(attachment_id)?;
        let pump_token = self
            .inner
            .next_cov_pump_token
            .fetch_add(1, Ordering::Relaxed);
        let (cancel, mut cancelled) = watch::channel(false);
        self.inner.cov_pumps.lock().await.insert(
            attachment_id,
            CovPumpControl {
                cancel,
                attachment_epoch,
                pump_token,
            },
        );
        let weak = Arc::downgrade(&self.inner);
        let accepted = self
            .inner
            .supervisor
            .spawn(move |mut global_stop| async move {
                loop {
                    let received = tokio::select! {
                        biased;
                        _ = global_stop.changed() => break,
                        _ = cancelled.changed() => break,
                        received = receiver.recv() => received,
                    };
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    let runtime = BacnetRuntime { inner };
                    match received {
                        Ok(notification) => {
                            let registry = runtime.inner.registry.read().await;
                            if registry.attachment_epoch(attachment_id) != Some(attachment_epoch) {
                                break;
                            }
                            runtime
                                .handle_cov_notification(attachment_id, notification)
                                .await;
                            drop(registry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            let registry = runtime.inner.registry.read().await;
                            if registry.attachment_epoch(attachment_id) != Some(attachment_epoch) {
                                break;
                            }
                            runtime.record_cov_lag(attachment_id, skipped).await;
                            drop(registry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }

                // A replaced pump may finish after its successor was installed.
                // Only remove the control that still belongs to this epoch.
                if let Some(inner) = weak.upgrade() {
                    let mut pumps = inner.cov_pumps.lock().await;
                    if pumps.get(&attachment_id).is_some_and(|control| {
                        control.attachment_epoch == attachment_epoch
                            && control.pump_token == pump_token
                    }) {
                        pumps.remove(&attachment_id);
                    }
                }
            })
            .await;
        if !accepted {
            let mut pumps = self.inner.cov_pumps.lock().await;
            if pumps.get(&attachment_id).is_some_and(|control| {
                control.attachment_epoch == attachment_epoch && control.pump_token == pump_token
            }) {
                pumps.remove(&attachment_id);
            }
            return Err(RuntimeError::stopped());
        }
        Ok(())
    }

    pub(super) async fn synchronize_cov_pumps(&self) -> Result<(), RuntimeError> {
        let _lifecycle = self.inner.cov_pump_lifecycle.lock().await;
        self.synchronize_cov_pumps_locked().await
    }

    pub(super) async fn synchronize_cov_pumps_locked(&self) -> Result<(), RuntimeError> {
        let expected = self.inner.registry.read().await.attachment_epochs();
        let expected_by_id = expected
            .iter()
            .copied()
            .collect::<std::collections::BTreeMap<_, _>>();
        {
            let mut pumps = self.inner.cov_pumps.lock().await;
            pumps.retain(|attachment_id, control| {
                let keep =
                    expected_by_id.get(attachment_id).copied() == Some(control.attachment_epoch);
                if !keep {
                    let _ = control.cancel.send(true);
                }
                keep
            });
        }
        for (attachment_id, _) in expected {
            self.ensure_cov_pump_locked(attachment_id).await?;
        }
        Ok(())
    }

    pub(super) async fn handle_cov_notification(
        &self,
        attachment_id: crate::AttachmentId,
        received: bacnet_client::client::ReceivedCOVNotification,
    ) {
        let notification = &received.notification;
        let mut specs = self
            .inner
            .observations
            .read()
            .await
            .for_attachment(attachment_id);
        specs.extend(
            self.inner
                .pending_observations
                .read()
                .await
                .values()
                .filter(|spec| spec.key.device.attachment_id == attachment_id)
                .cloned(),
        );
        let Some(spec) = specs.into_iter().find(|spec| {
            spec.key.device.device_instance
                == notification.initiating_device_identifier.instance_number()
                && u32::from(spec.key.object_type)
                    == notification
                        .monitored_object_identifier
                        .object_type()
                        .to_raw()
                && spec.key.object_instance
                    == notification.monitored_object_identifier.instance_number()
                && spec.key.subscriber_process_id == notification.subscriber_process_identifier
        }) else {
            self.inner
                .events
                .publish(
                    self.inner.generation.load(Ordering::Acquire),
                    Some(attachment_id),
                    EventKind::UnsolicitedCovNotification {
                        notification: crate::UnsolicitedCovNotification::from(&received),
                    },
                )
                .await;
            return;
        };
        let notification = received.notification;
        let values = notification
            .list_of_values
            .into_iter()
            .map(|value| CovValue {
                property_id: value.property_identifier.to_raw(),
                array_index: value.property_array_index,
                raw_value: value.value,
            })
            .collect::<Vec<_>>();
        if spec.suppress_poll_when_fresh {
            let now = std::time::Instant::now();
            let mut cache = self.inner.values.write().await;
            for value in &values {
                cache.insert(
                    spec.key.device,
                    &crate::PropertyRead {
                        input_index: 0,
                        object_type: spec.key.object_type,
                        object_instance: spec.key.object_instance,
                        property_id: value.property_id,
                        array_index: value.array_index,
                        value_category: "unknown".to_owned(),
                    },
                    value.raw_value.clone(),
                    now,
                );
            }
        }
        let duplicate = self
            .inner
            .cov_last
            .lock()
            .await
            .insert(spec.key, values.clone())
            .is_some_and(|previous| previous == values);
        if !duplicate {
            self.inner
                .events
                .publish(
                    self.inner.generation.load(Ordering::Acquire),
                    Some(attachment_id),
                    EventKind::CovNotification {
                        key: spec.key,
                        time_remaining: notification.time_remaining,
                        values,
                    },
                )
                .await;
        }
    }

    pub(super) async fn record_cov_lag(&self, attachment_id: crate::AttachmentId, skipped: u64) {
        self.inner
            .cov_notification_lag_count
            .fetch_add(skipped, Ordering::Relaxed);
        self.inner
            .events
            .publish(
                self.inner.generation.load(Ordering::Acquire),
                Some(attachment_id),
                EventKind::CovNotificationLagged { skipped },
            )
            .await;
    }

    pub(super) async fn ensure_i_am_pump(
        &self,
        attachment_id: crate::AttachmentId,
    ) -> Result<(), RuntimeError> {
        if self
            .inner
            .i_am_pumps
            .lock()
            .await
            .contains_key(&attachment_id)
        {
            return Ok(());
        }
        let (attachment_epoch, mut receiver) = self
            .inner
            .registry
            .read()
            .await
            .i_am_receiver(attachment_id)?;
        let (cancel, mut cancelled) = watch::channel(false);
        self.inner.i_am_pumps.lock().await.insert(
            attachment_id,
            IAmPumpControl {
                cancel,
                attachment_epoch,
            },
        );
        let weak = Arc::downgrade(&self.inner);
        let _ = self
            .inner
            .supervisor
            .spawn(move |mut global_stop| async move {
                loop {
                    let received = tokio::select! {
                        biased;
                        _ = global_stop.changed() => return,
                        _ = cancelled.changed() => return,
                        received = receiver.recv() => received,
                    };
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    let runtime = BacnetRuntime { inner };
                    match received {
                        Ok(observation) => {
                            if observation.kind == bacnet_client::client::DeviceEventKind::Lost {
                                continue;
                            }
                            let registry = runtime.inner.registry.read().await;
                            if registry.attachment_epoch(attachment_id) != Some(attachment_epoch) {
                                return;
                            }
                            runtime
                                .inner
                                .events
                                .publish(
                                    runtime.inner.generation.load(Ordering::Acquire),
                                    Some(attachment_id),
                                    EventKind::IAmObservation {
                                        observation: observation.into(),
                                    },
                                )
                                .await;
                            drop(registry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            let registry = runtime.inner.registry.read().await;
                            if registry.attachment_epoch(attachment_id) != Some(attachment_epoch) {
                                return;
                            }
                            runtime.record_i_am_lag(attachment_id, skipped).await;
                            drop(registry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            })
            .await;
        Ok(())
    }

    pub(super) async fn synchronize_i_am_pumps(&self) -> Result<(), RuntimeError> {
        let expected = self.inner.registry.read().await.attachment_epochs();
        let expected_by_id = expected
            .iter()
            .copied()
            .collect::<std::collections::BTreeMap<_, _>>();
        {
            let mut pumps = self.inner.i_am_pumps.lock().await;
            pumps.retain(|attachment_id, control| {
                let keep =
                    expected_by_id.get(attachment_id).copied() == Some(control.attachment_epoch);
                if !keep {
                    let _ = control.cancel.send(true);
                }
                keep
            });
        }
        for (attachment_id, _) in expected {
            self.ensure_i_am_pump(attachment_id).await?;
        }
        Ok(())
    }

    pub(super) async fn record_i_am_lag(&self, attachment_id: crate::AttachmentId, skipped: u64) {
        self.inner
            .i_am_observation_lag_count
            .fetch_add(skipped, Ordering::Relaxed);
        self.inner
            .events
            .publish(
                self.inner.generation.load(Ordering::Acquire),
                Some(attachment_id),
                EventKind::IAmObservationLagged { skipped },
            )
            .await;
    }

    pub(super) async fn drop_cov_attachment(&self, attachment_id: crate::AttachmentId) {
        let removed = self
            .inner
            .observations
            .write()
            .await
            .remove_attachment(attachment_id);
        for spec in removed {
            self.stop_cov_renewal(spec.key).await;
            self.inner.cov_last.lock().await.remove(&spec.key);
        }
        {
            let _lifecycle = self.inner.cov_pump_lifecycle.lock().await;
            if let Some(control) = self.inner.cov_pumps.lock().await.remove(&attachment_id) {
                let _ = control.cancel.send(true);
            }
        }
    }

    pub(super) async fn refresh_cov_attachment(&self, attachment_id: crate::AttachmentId) {
        {
            let _lifecycle = self.inner.cov_pump_lifecycle.lock().await;
            if let Some(control) = self.inner.cov_pumps.lock().await.remove(&attachment_id) {
                let _ = control.cancel.send(true);
            }
            if self.ensure_cov_pump_locked(attachment_id).await.is_err() {
                return;
            }
        }
        let specs = self
            .inner
            .observations
            .read()
            .await
            .for_attachment(attachment_id);
        for spec in specs {
            self.stop_cov_renewal(spec.key).await;
            match self.set_cov_subscription(&spec, true).await {
                Ok(()) => self.start_cov_renewal(spec).await,
                Err(error) => {
                    self.inner
                        .events
                        .publish(
                            self.inner.generation.load(Ordering::Acquire),
                            Some(attachment_id),
                            EventKind::CovRenewalFailed {
                                key: spec.key,
                                code: error.code,
                            },
                        )
                        .await;
                }
            }
        }
    }
}
