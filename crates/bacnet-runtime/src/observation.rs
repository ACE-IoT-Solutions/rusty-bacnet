use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::{DeviceKey, RuntimeError};

/// Stable identity of one managed COV subscription.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservationKey {
    /// Exact attachment-qualified device.
    pub device: DeviceKey,
    /// Monitored BACnet object type, including proprietary values.
    pub object_type: u16,
    /// Monitored BACnet object instance.
    pub object_instance: u32,
    /// Subscriber process identifier sent on the wire.
    pub subscriber_process_id: u32,
}

/// Desired finite managed COV subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSpec {
    /// Stable subscription identity.
    pub key: ObservationKey,
    /// Request confirmed notifications when true.
    pub confirmed: bool,
    /// Requested finite lifetime in seconds.
    pub lifetime_seconds: u32,
    /// Renew this long before the known expiry.
    pub renewal_margin: Duration,
    /// Permit fresh COV values to suppress scheduled polling.
    pub suppress_poll_when_fresh: bool,
}

impl ObservationSpec {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if self.lifetime_seconds == 0 {
            return Err(RuntimeError::invalid_config(
                "COV lifetime_seconds must be non-zero",
            ));
        }
        if self.renewal_margin >= Duration::from_secs(u64::from(self.lifetime_seconds)) {
            return Err(RuntimeError::invalid_config(
                "COV renewal_margin must be shorter than lifetime",
            ));
        }
        bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::from_raw(u32::from(self.key.object_type)),
            self.key.object_instance,
        )
        .map(|_| ())
        .map_err(|error| RuntimeError::invalid_config(error.to_string()))
    }
}

/// Complete revisioned desired observation state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationPlan {
    /// Monotonically increasing complete-plan revision.
    pub revision: u64,
    /// Desired subscriptions.
    pub subscriptions: Vec<ObservationSpec>,
}

/// Transactional change set between observation revisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObservationChanges {
    pub(crate) revision: u64,
    pub(crate) desired: BTreeMap<ObservationKey, ObservationSpec>,
    pub(crate) added: Vec<ObservationSpec>,
    pub(crate) updated: Vec<ObservationSpec>,
    pub(crate) removed: Vec<ObservationSpec>,
    pub(crate) idempotent: bool,
}

/// Result of applying a complete observation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationReport {
    /// Applied plan revision.
    pub revision: u64,
    /// Newly subscribed identities.
    pub added: Vec<ObservationKey>,
    /// Replaced identities.
    pub updated: Vec<ObservationKey>,
    /// Unsubscribed identities.
    pub removed: Vec<ObservationKey>,
    /// True for an identical replay of the current revision.
    pub idempotent: bool,
}

/// One raw property value delivered by a COV notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CovValue {
    /// Numeric property identifier.
    pub property_id: u32,
    /// Optional array index.
    pub array_index: Option<u32>,
    /// Raw application-tagged value bytes.
    pub raw_value: Vec<u8>,
}

/// Revision and desired-state registry; network effects are staged by the runtime.
#[derive(Debug, Default)]
pub(crate) struct ObservationRegistry {
    revision: u64,
    subscriptions: BTreeMap<ObservationKey, ObservationSpec>,
}

impl ObservationRegistry {
    pub(crate) fn diff(&self, plan: ObservationPlan) -> Result<ObservationChanges, RuntimeError> {
        let mut desired = BTreeMap::new();
        for subscription in plan.subscriptions {
            subscription.validate()?;
            let key = subscription.key;
            if desired.insert(key, subscription).is_some() {
                return Err(RuntimeError::invalid_config(format!(
                    "duplicate COV subscription {key:?}"
                )));
            }
        }
        if plan.revision < self.revision
            || (plan.revision == self.revision && desired != self.subscriptions)
        {
            return Err(RuntimeError::stale_revision(plan.revision, self.revision));
        }
        if plan.revision == self.revision {
            return Ok(ObservationChanges {
                revision: plan.revision,
                desired,
                added: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                idempotent: true,
            });
        }
        let current_keys: BTreeSet<_> = self.subscriptions.keys().copied().collect();
        let desired_keys: BTreeSet<_> = desired.keys().copied().collect();
        let added = desired_keys
            .difference(&current_keys)
            .map(|key| desired[key].clone())
            .collect();
        let removed = current_keys
            .difference(&desired_keys)
            .map(|key| self.subscriptions[key].clone())
            .collect();
        let updated = desired_keys
            .intersection(&current_keys)
            .filter(|key| desired[key] != self.subscriptions[key])
            .map(|key| desired[key].clone())
            .collect();
        Ok(ObservationChanges {
            revision: plan.revision,
            desired,
            added,
            updated,
            removed,
            idempotent: false,
        })
    }

    pub(crate) fn commit(&mut self, changes: ObservationChanges) -> ObservationReport {
        let report = ObservationReport {
            revision: changes.revision,
            added: changes.added.iter().map(|spec| spec.key).collect(),
            updated: changes.updated.iter().map(|spec| spec.key).collect(),
            removed: changes.removed.iter().map(|spec| spec.key).collect(),
            idempotent: changes.idempotent,
        };
        self.revision = changes.revision;
        self.subscriptions = changes.desired;
        report
    }

    pub(crate) fn get(&self, key: ObservationKey) -> Option<&ObservationSpec> {
        self.subscriptions.get(&key)
    }

    pub(crate) fn for_attachment(
        &self,
        attachment_id: crate::AttachmentId,
    ) -> Vec<ObservationSpec> {
        self.subscriptions
            .values()
            .filter(|spec| spec.key.device.attachment_id == attachment_id)
            .cloned()
            .collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.subscriptions.len()
    }

    pub(crate) fn remove_attachment(
        &mut self,
        attachment_id: crate::AttachmentId,
    ) -> Vec<ObservationSpec> {
        let removed = self.for_attachment(attachment_id);
        self.subscriptions
            .retain(|key, _| key.device.attachment_id != attachment_id);
        if !removed.is_empty() {
            self.revision += 1;
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{AttachmentId, DeviceKey, ErrorCode};

    use super::{ObservationKey, ObservationPlan, ObservationRegistry, ObservationSpec};

    fn spec(instance: u32, confirmed: bool) -> ObservationSpec {
        ObservationSpec {
            key: ObservationKey {
                device: DeviceKey {
                    attachment_id: AttachmentId::from(1),
                    device_instance: 100,
                },
                object_type: 0,
                object_instance: instance,
                subscriber_process_id: instance + 1000,
            },
            confirmed,
            lifetime_seconds: 60,
            renewal_margin: Duration::from_secs(10),
            suppress_poll_when_fresh: true,
        }
    }

    #[test]
    fn revisioned_diff_is_deterministic_idempotent_and_stale_safe() {
        let mut registry = ObservationRegistry::default();
        let first = registry
            .diff(ObservationPlan {
                revision: 1,
                subscriptions: vec![spec(2, false), spec(1, false)],
            })
            .unwrap();
        assert_eq!(first.added.len(), 2);
        registry.commit(first);
        assert!(
            registry
                .commit(
                    registry
                        .diff(ObservationPlan {
                            revision: 1,
                            subscriptions: vec![spec(1, false), spec(2, false)],
                        })
                        .unwrap()
                )
                .idempotent
        );
        let second = registry
            .diff(ObservationPlan {
                revision: 2,
                subscriptions: vec![spec(1, true), spec(3, false)],
            })
            .unwrap();
        assert_eq!(second.updated[0].key.object_instance, 1);
        assert_eq!(second.added[0].key.object_instance, 3);
        assert_eq!(second.removed[0].key.object_instance, 2);
        assert_eq!(
            registry
                .diff(ObservationPlan {
                    revision: 0,
                    subscriptions: Vec::new(),
                })
                .unwrap_err()
                .code,
            ErrorCode::StaleRevision
        );
    }
}
