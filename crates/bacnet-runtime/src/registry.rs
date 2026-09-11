use std::collections::{BTreeMap, BTreeSet};

use futures_util::future::join_all;

use crate::transport::{AttachmentDiscovery, AttachmentTopologyResult};
use crate::{
    AttachmentConfig, AttachmentHealth, AttachmentId, AttachmentState, DiscoveryRequest,
    RuntimeError, RuntimeTransport, TopologyRequest,
};
use bacnet_client::client::ReceivedCOVNotification;
use bacnet_client::discovery::IAmEvent;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

static NEXT_ATTACHMENT_EPOCH: AtomicU64 = AtomicU64::new(1);

fn apply_foreign_registration_health(
    health: &mut AttachmentHealth,
    registration: Option<&crate::ForeignDeviceRegistrationStatus>,
) {
    if health.state != AttachmentState::Running {
        return;
    }
    if health.last_error == Some(crate::ErrorCode::ForeignDeviceRegistrationFailed) {
        health.last_error = None;
    }
    if registration.is_some_and(|status| {
        matches!(
            status.state,
            crate::ForeignDeviceRegistrationState::Rejected
                | crate::ForeignDeviceRegistrationState::Expired
        )
    }) {
        // Registration is an attachment-level degradation, not a transport
        // failure: unicast remains usable while BBMD forwarding is unavailable.
        health.last_error = Some(crate::ErrorCode::ForeignDeviceRegistrationFailed);
    }
}

#[derive(Debug)]
struct AttachmentEntry {
    config: AttachmentConfig,
    health: AttachmentHealth,
    transport: RuntimeTransport,
    epoch: u64,
    initial_cov_receiver: std::sync::Mutex<Option<broadcast::Receiver<ReceivedCOVNotification>>>,
    initial_i_am_receiver: std::sync::Mutex<Option<broadcast::Receiver<IAmEvent>>>,
}

impl AttachmentEntry {
    async fn start(
        config: &AttachmentConfig,
        cov_channel_capacity: usize,
    ) -> Result<Self, RuntimeError> {
        let mut transport = RuntimeTransport::start(config, cov_channel_capacity).await?;
        // Reserve both broadcast receivers immediately after the transport
        // starts so frames received before runtime pumps attach stay queued.
        let initial_cov_receiver = transport
            .take_initial_cov_receiver()
            .unwrap_or_else(|| transport.cov_receiver());
        let initial_i_am_receiver = transport.take_initial_i_am_receiver();
        Ok(Self {
            config: config.clone(),
            health: AttachmentHealth {
                id: config.id,
                label: config.label.clone(),
                state: AttachmentState::Running,
                last_error: None,
                foreign_device_registration: None,
            },
            transport,
            epoch: NEXT_ATTACHMENT_EPOCH.fetch_add(1, Ordering::Relaxed),
            initial_cov_receiver: std::sync::Mutex::new(Some(initial_cov_receiver)),
            initial_i_am_receiver: std::sync::Mutex::new(initial_i_am_receiver),
        })
    }

    async fn stop(&mut self) -> Result<(), RuntimeError> {
        self.health.state = AttachmentState::Stopping;
        let result = self.transport.stop(self.config.id).await;
        match &result {
            Ok(()) => self.health.state = AttachmentState::Stopped,
            Err(error) => {
                self.health.state = AttachmentState::Failed;
                self.health.last_error = Some(error.code);
            }
        }
        result
    }
}

#[derive(Debug)]
pub(crate) struct RegistryReconcile {
    pub(crate) added: Vec<AttachmentId>,
    pub(crate) updated: Vec<AttachmentId>,
    pub(crate) removed: Vec<AttachmentId>,
}

#[derive(Debug)]
pub(crate) struct AttachmentRegistry {
    entries: BTreeMap<AttachmentId, AttachmentEntry>,
    order: Vec<AttachmentId>,
    cov_channel_capacity: usize,
}

impl AttachmentRegistry {
    pub(crate) async fn start(
        configs: &[AttachmentConfig],
        cov_channel_capacity: usize,
    ) -> Result<Self, RuntimeError> {
        let mut registry = Self {
            entries: BTreeMap::new(),
            order: Vec::with_capacity(configs.len()),
            cov_channel_capacity,
        };
        for config in configs {
            let entry = match AttachmentEntry::start(config, cov_channel_capacity).await {
                Ok(entry) => entry,
                Err(error) => {
                    let _ = registry.stop_all().await;
                    return Err(error);
                }
            };
            registry.order.push(config.id);
            registry.entries.insert(config.id, entry);
        }
        Ok(registry)
    }

    pub(crate) fn health(&self) -> Vec<AttachmentHealth> {
        self.order
            .iter()
            .filter_map(|id| {
                self.entries.get(id).map(|entry| {
                    let mut health = entry.health.clone();
                    let registration = entry.transport.foreign_device_registration_status();
                    if health.state == AttachmentState::Running {
                        if let Some(error) = entry.transport.health_error() {
                            health.state = AttachmentState::Failed;
                            health.last_error = Some(error);
                        }
                    }
                    apply_foreign_registration_health(&mut health, registration.as_ref());
                    health.foreign_device_registration = registration;
                    health
                })
            })
            .collect()
    }

    pub(crate) fn configs(&self) -> Vec<AttachmentConfig> {
        self.order
            .iter()
            .filter_map(|id| self.entries.get(id).map(|entry| entry.config.clone()))
            .collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn background_task_count(&self) -> usize {
        self.entries
            .values()
            .map(|entry| entry.transport.background_task_count())
            .sum()
    }

    pub(crate) fn transport(
        &self,
        attachment_id: AttachmentId,
    ) -> Result<&RuntimeTransport, RuntimeError> {
        self.entries
            .get(&attachment_id)
            .map(|entry| &entry.transport)
            .ok_or_else(|| RuntimeError::attachment_not_found(attachment_id))
    }

    pub(crate) fn cov_receiver(
        &self,
        attachment_id: AttachmentId,
    ) -> Result<(u64, broadcast::Receiver<ReceivedCOVNotification>), RuntimeError> {
        let entry = self
            .entries
            .get(&attachment_id)
            .ok_or_else(|| RuntimeError::attachment_not_found(attachment_id))?;
        let receiver = entry
            .initial_cov_receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| entry.transport.cov_receiver());
        Ok((entry.epoch, receiver))
    }

    pub(crate) fn i_am_receiver(
        &self,
        attachment_id: AttachmentId,
    ) -> Result<(u64, broadcast::Receiver<IAmEvent>), RuntimeError> {
        let entry = self
            .entries
            .get(&attachment_id)
            .ok_or_else(|| RuntimeError::attachment_not_found(attachment_id))?;
        let receiver = entry
            .initial_i_am_receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| entry.transport.i_am_receiver());
        Ok((entry.epoch, receiver))
    }

    pub(crate) fn attachment_epoch(&self, attachment_id: AttachmentId) -> Option<u64> {
        self.entries.get(&attachment_id).map(|entry| entry.epoch)
    }

    pub(crate) fn attachment_epochs(&self) -> Vec<(AttachmentId, u64)> {
        self.order
            .iter()
            .filter_map(|id| self.entries.get(id).map(|entry| (*id, entry.epoch)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn has_reserved_i_am_receiver(&self, attachment_id: AttachmentId) -> bool {
        self.entries.get(&attachment_id).is_some_and(|entry| {
            entry
                .initial_i_am_receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some()
        })
    }

    #[cfg(test)]
    pub(crate) fn has_reserved_cov_receiver(&self, attachment_id: AttachmentId) -> bool {
        self.entries.get(&attachment_id).is_some_and(|entry| {
            entry
                .initial_cov_receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some()
        })
    }

    pub(crate) async fn discover(
        &self,
        request: &DiscoveryRequest,
    ) -> Vec<(AttachmentId, AttachmentDiscovery)> {
        let selected = if request.attachment_ids.is_empty() {
            self.order.clone()
        } else {
            request.attachment_ids.clone()
        };
        let futures = selected.into_iter().map(|id| async move {
            let Some(entry) = self.entries.get(&id) else {
                return (
                    id,
                    AttachmentDiscovery {
                        devices: Vec::new(),
                        routers: Vec::new(),
                        errors: vec![RuntimeError::attachment_not_found(id)],
                    },
                );
            };
            (id, entry.transport.discover(id, request).await)
        });
        join_all(futures).await
    }

    pub(crate) async fn topology(
        &self,
        request: &TopologyRequest,
    ) -> Vec<(AttachmentId, AttachmentTopologyResult)> {
        let selected = if request.attachment_ids.is_empty() {
            self.order.clone()
        } else {
            request.attachment_ids.clone()
        };
        let futures = selected.into_iter().map(|id| async move {
            let Some(entry) = self.entries.get(&id) else {
                return (
                    id,
                    AttachmentTopologyResult {
                        local_mac: Vec::new(),
                        bbmds: Vec::new(),
                        routers: Vec::new(),
                        truncated: false,
                        errors: vec![RuntimeError::attachment_not_found(id)],
                    },
                );
            };
            let seeds = request
                .bbmd_seeds
                .iter()
                .filter(|seed| seed.attachment_id == id)
                .map(|seed| seed.mac.clone())
                .collect();
            (
                id,
                entry
                    .transport
                    .topology(
                        id,
                        seeds,
                        request.include_fdt,
                        request.max_bbmds_per_attachment,
                    )
                    .await,
            )
        });
        join_all(futures).await
    }

    #[cfg(test)]
    pub(crate) async fn seed_device(&self, attachment_id: AttachmentId, instance: u32, mac: &[u8]) {
        self.entries[&attachment_id]
            .transport
            .seed_device(instance, mac)
            .await
            .unwrap();
    }

    pub(crate) async fn reconcile(
        &mut self,
        desired: &[AttachmentConfig],
    ) -> Result<RegistryReconcile, RuntimeError> {
        let desired_by_id: BTreeMap<_, _> = desired
            .iter()
            .map(|config| (config.id, config.clone()))
            .collect();
        let existing_ids: BTreeSet<_> = self.entries.keys().copied().collect();
        let desired_ids: BTreeSet<_> = desired_by_id.keys().copied().collect();

        let added: Vec<_> = desired
            .iter()
            .filter(|config| !existing_ids.contains(&config.id))
            .map(|config| config.id)
            .collect();
        let removed: Vec<_> = self
            .order
            .iter()
            .filter(|id| !desired_ids.contains(id))
            .copied()
            .collect();
        let updated: Vec<_> = desired
            .iter()
            .filter(|config| {
                self.entries
                    .get(&config.id)
                    .is_some_and(|entry| entry.config != **config)
            })
            .map(|config| config.id)
            .collect();

        let mut staged: BTreeMap<AttachmentId, AttachmentEntry> = BTreeMap::new();
        let mut same_bind_replaced: Vec<AttachmentConfig> = Vec::new();

        for id in &added {
            let config = &desired_by_id[id];
            match AttachmentEntry::start(config, self.cov_channel_capacity).await {
                Ok(entry) => {
                    staged.insert(*id, entry);
                }
                Err(error) => {
                    Self::stop_entries(&mut staged).await;
                    return Err(error);
                }
            }
        }

        for id in &updated {
            let desired_config = &desired_by_id[id];
            let current_config = self.entries[id].config.clone();
            if current_config.transport == desired_config.transport {
                continue;
            }

            if RuntimeTransport::must_stop_before_replacement(
                &current_config.transport,
                &desired_config.transport,
            ) {
                let mut current = self
                    .entries
                    .remove(id)
                    .expect("updated attachment must exist");
                if let Err(error) = current.stop().await {
                    self.entries.insert(*id, current);
                    Self::stop_entries(&mut staged).await;
                    return Err(error);
                }
                match AttachmentEntry::start(desired_config, self.cov_channel_capacity).await {
                    Ok(replacement) => {
                        self.entries.insert(*id, replacement);
                        same_bind_replaced.push(current_config);
                    }
                    Err(error) => {
                        let rollback =
                            AttachmentEntry::start(&current_config, self.cov_channel_capacity)
                                .await;
                        if let Ok(restored) = rollback {
                            self.entries.insert(*id, restored);
                        } else if let Err(rollback) = rollback {
                            Self::stop_entries(&mut staged).await;
                            return Err(RuntimeError::rollback_failed(*id, &error, &rollback));
                        }
                        Self::rollback_same_bind(self, &same_bind_replaced, &mut staged, &error)
                            .await?;
                        return Err(error);
                    }
                }
            } else {
                match AttachmentEntry::start(desired_config, self.cov_channel_capacity).await {
                    Ok(replacement) => {
                        staged.insert(*id, replacement);
                    }
                    Err(error) => {
                        Self::rollback_same_bind(self, &same_bind_replaced, &mut staged, &error)
                            .await?;
                        return Err(error);
                    }
                }
            }
        }

        for id in &updated {
            let desired_config = &desired_by_id[id];
            if self.entries[id].config.transport == desired_config.transport {
                let entry = self
                    .entries
                    .get_mut(id)
                    .expect("updated attachment must exist");
                entry.config = desired_config.clone();
                entry.health.label = desired_config.label.clone();
                continue;
            }

            if let Some(replacement) = staged.remove(id) {
                let mut current = self
                    .entries
                    .remove(id)
                    .expect("replacement attachment must exist");
                if let Err(error) = current.stop().await {
                    self.entries.insert(*id, current);
                    let mut replacement = replacement;
                    let _ = replacement.stop().await;
                    Self::rollback_same_bind(self, &same_bind_replaced, &mut staged, &error)
                        .await?;
                    return Err(error);
                }
                self.entries.insert(*id, replacement);
            }
        }

        for id in &added {
            self.entries.insert(
                *id,
                staged
                    .remove(id)
                    .expect("successfully staged addition must exist"),
            );
        }

        for id in &removed {
            let mut entry = self
                .entries
                .remove(id)
                .expect("removed attachment must exist");
            if let Err(error) = entry.stop().await {
                self.entries.insert(*id, entry);
                return Err(error);
            }
        }

        self.order = desired.iter().map(|config| config.id).collect();
        Ok(RegistryReconcile {
            added,
            updated,
            removed,
        })
    }

    async fn rollback_same_bind(
        registry: &mut Self,
        prior_configs: &[AttachmentConfig],
        staged: &mut BTreeMap<AttachmentId, AttachmentEntry>,
        original: &RuntimeError,
    ) -> Result<(), RuntimeError> {
        Self::stop_entries(staged).await;
        for prior in prior_configs.iter().rev() {
            if let Some(mut replacement) = registry.entries.remove(&prior.id) {
                let _ = replacement.stop().await;
            }
            match AttachmentEntry::start(prior, registry.cov_channel_capacity).await {
                Ok(restored) => {
                    registry.entries.insert(prior.id, restored);
                }
                Err(rollback) => {
                    return Err(RuntimeError::rollback_failed(prior.id, original, &rollback));
                }
            }
        }
        Ok(())
    }

    async fn stop_entries(entries: &mut BTreeMap<AttachmentId, AttachmentEntry>) {
        for entry in entries.values_mut() {
            let _ = entry.stop().await;
        }
        entries.clear();
    }

    pub(crate) async fn stop_all(&mut self) -> Result<(), RuntimeError> {
        let ids = self.entries.keys().copied().collect::<Vec<_>>();
        let mut first_error = None;
        for id in ids {
            let result = self
                .entries
                .get_mut(&id)
                .expect("collected attachment must remain present")
                .stop()
                .await;
            match result {
                Ok(()) => {
                    self.entries.remove(&id);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        self.order.retain(|id| self.entries.contains_key(id));
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod registration_health_tests {
    use super::apply_foreign_registration_health;
    use crate::{
        AttachmentHealth, AttachmentId, AttachmentState, ErrorCode, ForeignDeviceRegistrationState,
        ForeignDeviceRegistrationStatus,
    };
    use bacnet_types::enums::BvlcResultCode;

    fn attachment(id: u128, label: &str) -> AttachmentHealth {
        AttachmentHealth {
            id: AttachmentId::from(id),
            label: label.to_owned(),
            state: AttachmentState::Running,
            last_error: None,
            foreign_device_registration: None,
        }
    }

    fn status(state: ForeignDeviceRegistrationState) -> ForeignDeviceRegistrationStatus {
        ForeignDeviceRegistrationStatus {
            state,
            last_result_code: Some(if state == ForeignDeviceRegistrationState::Rejected {
                BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
            } else {
                BvlcResultCode::SUCCESSFUL_COMPLETION
            }),
            seconds_to_renewal: None,
        }
    }

    #[test]
    fn rejected_expired_and_recovered_registration_preserves_order_and_running_state() {
        let mut health = vec![attachment(2, "foreign"), attachment(1, "normal")];

        let rejected = status(ForeignDeviceRegistrationState::Rejected);
        apply_foreign_registration_health(&mut health[0], Some(&rejected));
        apply_foreign_registration_health(&mut health[1], None);
        assert_eq!(
            health.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![AttachmentId::from(2), AttachmentId::from(1)]
        );
        assert_eq!(health[0].state, AttachmentState::Running);
        assert_eq!(
            health[0].last_error,
            Some(ErrorCode::ForeignDeviceRegistrationFailed)
        );
        assert_eq!(health[1].last_error, None);

        let expired = status(ForeignDeviceRegistrationState::Expired);
        apply_foreign_registration_health(&mut health[0], Some(&expired));
        assert_eq!(health[0].state, AttachmentState::Running);
        assert_eq!(
            health[0].last_error,
            Some(ErrorCode::ForeignDeviceRegistrationFailed)
        );

        let recovered = status(ForeignDeviceRegistrationState::Registered);
        apply_foreign_registration_health(&mut health[0], Some(&recovered));
        assert_eq!(health[0].state, AttachmentState::Running);
        assert_eq!(health[0].last_error, None);
    }

    #[test]
    fn registration_status_does_not_mask_a_failed_transport() {
        let mut health = attachment(3, "failed-transport");
        health.state = AttachmentState::Failed;
        health.last_error = Some(ErrorCode::DeviceUnavailable);
        let recovered = status(ForeignDeviceRegistrationState::Registered);

        apply_foreign_registration_health(&mut health, Some(&recovered));

        assert_eq!(health.state, AttachmentState::Failed);
        assert_eq!(health.last_error, Some(ErrorCode::DeviceUnavailable));
    }
}
