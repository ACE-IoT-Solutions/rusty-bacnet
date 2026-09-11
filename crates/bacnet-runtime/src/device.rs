use std::collections::{BTreeMap, BTreeSet};

use crate::AttachmentId;

/// Transport-qualified BACnet device identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceKey {
    /// Attachment that owns this observation.
    pub attachment_id: AttachmentId,
    /// BACnet Device object instance.
    pub device_instance: u32,
}

/// Lossless transport-native path to a BACnet device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevicePath {
    /// Device directly reachable at one data-link MAC.
    Direct {
        /// Transport-native MAC bytes: B/IP six, MS/TP one, SC six.
        mac: Vec<u8>,
    },
    /// Device reachable through an ingress router and destination network.
    Routed {
        /// Immediate router MAC on the owning attachment.
        ingress_mac: Vec<u8>,
        /// Destination BACnet network number.
        dnet: u16,
        /// Destination data-link address, including leading zero bytes.
        dadr: Vec<u8>,
    },
}

/// One attachment-qualified device observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceObservation {
    /// Stable device identity.
    pub key: DeviceKey,
    /// Current direct or routed path.
    pub path: DevicePath,
    /// Vendor identifier from I-Am.
    pub vendor_id: u16,
    /// Maximum APDU accepted by the remote device.
    pub max_apdu_length: u16,
    /// Monotonic revision advanced when this observation changes.
    pub revision: u64,
}

/// Persisted attachment-qualified device path restored before BACnet I/O.
///
/// This is deliberately transport-native: callers persist the exact MAC/DNET/DADR
/// bytes returned by discovery and restore them without reconstructing APDUs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedDevice {
    /// Stable attachment-qualified device identity.
    pub key: DeviceKey,
    /// Exact direct or routed path returned by discovery.
    pub path: DevicePath,
    /// Last known vendor identifier, or zero when unavailable.
    pub vendor_id: u16,
    /// Last known maximum accepted APDU length.
    pub max_apdu_length: u16,
}

/// Result of atomically restoring a set of persisted device paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceRestoreReport {
    /// Paths not previously present in the runtime index.
    pub added: usize,
    /// Existing paths or capabilities replaced by persisted state.
    pub updated: usize,
    /// Persisted entries already identical to runtime state.
    pub unchanged: usize,
    /// Resulting global device-index revision.
    pub index_revision: u64,
}

/// Selected and alternate observations for one device instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSelection {
    /// Device instance shared by the observations.
    pub device_instance: u32,
    /// Deterministically selected healthy observation, if any.
    pub selected: Option<DeviceObservation>,
    /// Every non-selected observation in configuration order.
    pub alternates: Vec<DeviceObservation>,
}

/// Kind of device-index mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceIndexChangeKind {
    /// First observation for this attachment/device key.
    Added,
    /// Existing observation changed path or capabilities.
    Updated,
    /// Identical observation refreshed without a revision change.
    Unchanged,
    /// Attachment removal deleted the observation.
    Removed,
}

/// Result of one device-index mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceIndexChange {
    /// Changed key.
    pub key: DeviceKey,
    /// Mutation kind.
    pub kind: DeviceIndexChangeKind,
    /// Resulting global index revision.
    pub index_revision: u64,
    /// Resulting observation revision, or final revision when removed.
    pub observation_revision: u64,
}

/// Cross-attachment device index retaining duplicate and alternate observations.
#[derive(Debug, Default)]
pub struct DeviceIndex {
    observations: BTreeMap<u32, BTreeMap<AttachmentId, DeviceObservation>>,
    attachment_order: Vec<AttachmentId>,
    healthy: BTreeSet<AttachmentId>,
    preferred: BTreeMap<u32, AttachmentId>,
    revision: u64,
}

impl DeviceIndex {
    /// Creates an empty index with deterministic attachment order.
    pub fn new(attachment_order: Vec<AttachmentId>) -> Self {
        Self {
            attachment_order,
            ..Self::default()
        }
    }

    /// Current global index revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Number of distinct device instances observed.
    pub fn device_count(&self) -> usize {
        self.observations.len()
    }

    /// Number of attachment-qualified observations, including duplicates.
    pub fn observation_count(&self) -> usize {
        self.observations.values().map(BTreeMap::len).sum()
    }

    /// Replaces configuration order used for unqualified selection.
    pub fn set_attachment_order(&mut self, order: Vec<AttachmentId>) {
        self.attachment_order = order;
    }

    /// Marks an attachment healthy or unhealthy for selection purposes.
    pub fn set_attachment_health(&mut self, attachment_id: AttachmentId, healthy: bool) {
        if healthy {
            self.healthy.insert(attachment_id);
        } else {
            self.healthy.remove(&attachment_id);
        }
    }

    /// Sets or clears the persisted preferred attachment for one device instance.
    pub fn set_preferred(&mut self, device_instance: u32, attachment_id: Option<AttachmentId>) {
        if let Some(attachment_id) = attachment_id {
            self.preferred.insert(device_instance, attachment_id);
        } else {
            self.preferred.remove(&device_instance);
        }
    }

    /// Inserts or updates an observation while preserving its monotonic revision.
    pub fn upsert(&mut self, mut observation: DeviceObservation) -> DeviceIndexChange {
        let by_attachment = self
            .observations
            .entry(observation.key.device_instance)
            .or_default();
        let current = by_attachment.get(&observation.key.attachment_id);
        let (kind, observation_revision) = match current {
            None => (DeviceIndexChangeKind::Added, 1),
            Some(current)
                if current.path == observation.path
                    && current.vendor_id == observation.vendor_id
                    && current.max_apdu_length == observation.max_apdu_length =>
            {
                return DeviceIndexChange {
                    key: observation.key,
                    kind: DeviceIndexChangeKind::Unchanged,
                    index_revision: self.revision,
                    observation_revision: current.revision,
                };
            }
            Some(current) => (DeviceIndexChangeKind::Updated, current.revision + 1),
        };
        observation.revision = observation_revision;
        by_attachment.insert(observation.key.attachment_id, observation.clone());
        self.revision += 1;
        DeviceIndexChange {
            key: observation.key,
            kind,
            index_revision: self.revision,
            observation_revision,
        }
    }

    /// Returns exactly the explicitly qualified observation.
    pub fn get(&self, key: DeviceKey) -> Option<&DeviceObservation> {
        self.observations
            .get(&key.device_instance)?
            .get(&key.attachment_id)
    }

    /// Selects one healthy observation and retains every alternate.
    pub fn selection(&self, device_instance: u32) -> DeviceSelection {
        let Some(observations) = self.observations.get(&device_instance) else {
            return DeviceSelection {
                device_instance,
                selected: None,
                alternates: Vec::new(),
            };
        };
        let preferred = self
            .preferred
            .get(&device_instance)
            .copied()
            .filter(|id| self.healthy.contains(id) && observations.contains_key(id));
        let selected_id = preferred.or_else(|| {
            self.attachment_order
                .iter()
                .copied()
                .find(|id| self.healthy.contains(id) && observations.contains_key(id))
        });
        let selected = selected_id.and_then(|id| observations.get(&id).cloned());
        let alternates = self
            .attachment_order
            .iter()
            .filter(|id| Some(**id) != selected_id)
            .filter_map(|id| observations.get(id).cloned())
            .chain(
                observations
                    .iter()
                    .filter(|(id, _)| !self.attachment_order.contains(id))
                    .filter(|(id, _)| Some(**id) != selected_id)
                    .map(|(_, observation)| observation.clone()),
            )
            .collect();
        DeviceSelection {
            device_instance,
            selected,
            alternates,
        }
    }

    /// Removes all state owned by an attachment and returns removal changes.
    pub fn remove_attachment(&mut self, attachment_id: AttachmentId) -> Vec<DeviceIndexChange> {
        let mut changes = Vec::new();
        self.healthy.remove(&attachment_id);
        self.preferred
            .retain(|_, preferred| *preferred != attachment_id);
        for observations in self.observations.values_mut() {
            if let Some(removed) = observations.remove(&attachment_id) {
                self.revision += 1;
                changes.push(DeviceIndexChange {
                    key: removed.key,
                    kind: DeviceIndexChangeKind::Removed,
                    index_revision: self.revision,
                    observation_revision: removed.revision,
                });
            }
        }
        self.observations
            .retain(|_, observations| !observations.is_empty());
        changes
    }
}

#[cfg(test)]
mod tests {
    use crate::AttachmentId;

    use super::{DeviceIndex, DeviceIndexChangeKind, DeviceKey, DeviceObservation, DevicePath};

    fn observation(attachment: u128, path: DevicePath) -> DeviceObservation {
        DeviceObservation {
            key: DeviceKey {
                attachment_id: AttachmentId::from(attachment),
                device_instance: 100,
            },
            path,
            vendor_id: 42,
            max_apdu_length: 1476,
            revision: 999,
        }
    }

    #[test]
    fn paths_preserve_nondefault_ports_leading_zero_dadr_and_mstp_mac() {
        let direct = DevicePath::Direct {
            mac: vec![192, 0, 2, 10, 0xba, 0xc1],
        };
        let routed = DevicePath::Routed {
            ingress_mac: vec![198, 51, 100, 7, 0xba, 0xc2],
            dnet: 2200,
            dadr: vec![0, 0x7f, 1],
        };
        let mstp = DevicePath::Direct { mac: vec![5] };
        assert_eq!(direct, observation(1, direct.clone()).path);
        assert_eq!(routed, observation(1, routed.clone()).path);
        assert_eq!(mstp, observation(2, mstp.clone()).path);
    }

    #[test]
    fn duplicate_selection_uses_preference_then_configuration_order() {
        let first = AttachmentId::from(1);
        let second = AttachmentId::from(2);
        let mut index = DeviceIndex::new(vec![first, second]);
        index.set_attachment_health(first, true);
        index.set_attachment_health(second, true);
        index.upsert(observation(2, DevicePath::Direct { mac: vec![5] }));
        index.upsert(observation(
            1,
            DevicePath::Direct {
                mac: vec![192, 0, 2, 10, 0xba, 0xc1],
            },
        ));

        let selected = index.selection(100);
        assert_eq!(selected.selected.unwrap().key.attachment_id, first);
        assert_eq!(selected.alternates.len(), 1);

        index.set_preferred(100, Some(second));
        assert_eq!(
            index.selection(100).selected.unwrap().key.attachment_id,
            second
        );
        index.set_attachment_health(second, false);
        assert_eq!(
            index.selection(100).selected.unwrap().key.attachment_id,
            first
        );
    }

    #[test]
    fn path_change_advances_observation_and_index_revisions() {
        let mut index = DeviceIndex::new(vec![AttachmentId::from(1)]);
        let added = index.upsert(observation(
            1,
            DevicePath::Direct {
                mac: vec![192, 0, 2, 10, 0xba, 0xc0],
            },
        ));
        assert_eq!(added.kind, DeviceIndexChangeKind::Added);
        assert_eq!(added.observation_revision, 1);

        let unchanged = index.upsert(observation(
            1,
            DevicePath::Direct {
                mac: vec![192, 0, 2, 10, 0xba, 0xc0],
            },
        ));
        assert_eq!(unchanged.kind, DeviceIndexChangeKind::Unchanged);
        assert_eq!(unchanged.index_revision, 1);

        let updated = index.upsert(observation(
            1,
            DevicePath::Routed {
                ingress_mac: vec![192, 0, 2, 1, 0xba, 0xc0],
                dnet: 1001,
                dadr: vec![0, 5],
            },
        ));
        assert_eq!(updated.kind, DeviceIndexChangeKind::Updated);
        assert_eq!(updated.observation_revision, 2);
        assert_eq!(index.revision(), 2);
    }

    #[test]
    fn attachment_removal_deletes_only_owned_observations() {
        let first = AttachmentId::from(1);
        let second = AttachmentId::from(2);
        let mut index = DeviceIndex::new(vec![first, second]);
        index.upsert(observation(1, DevicePath::Direct { mac: vec![1] }));
        index.upsert(observation(2, DevicePath::Direct { mac: vec![2] }));

        let removed = index.remove_attachment(first);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].kind, DeviceIndexChangeKind::Removed);
        assert!(index
            .get(DeviceKey {
                attachment_id: first,
                device_instance: 100,
            })
            .is_none());
        assert!(index
            .get(DeviceKey {
                attachment_id: second,
                device_instance: 100,
            })
            .is_some());
    }
}
