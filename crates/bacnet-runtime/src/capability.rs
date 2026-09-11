use std::collections::BTreeMap;

use crate::DeviceKey;

/// Learned support state for a device feature.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Support {
    /// No result has been learned.
    #[default]
    Unknown,
    /// Device accepted the feature.
    Supported,
    /// Device definitively rejected the feature.
    Unsupported,
}

/// Successful object-list enumeration strategy learned for a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectListStrategy {
    /// Whole object list inside RPM.
    RpmWholeArray,
    /// Whole-array ReadProperty.
    RpWholeArray,
    /// Array count followed by bounded indexed reads.
    Indexed,
}

/// One attempt in the bounded object-list fallback sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectListAttempt {
    /// RPM containing the whole object-list property.
    RpmWholeArray,
    /// RP of the whole object-list property.
    RpWholeArray,
    /// RP of array index zero to obtain the count.
    IndexedCount,
}

/// Learned scan capabilities for one attachment-qualified device.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceCapabilities {
    /// ReadPropertyMultiple support.
    pub rpm: Support,
    /// Last successful object-list strategy.
    pub object_list: Option<ObjectListStrategy>,
    /// Largest property count observed to succeed in one RPM.
    pub safe_rpm_properties: Option<usize>,
    /// Monotonic capability revision.
    pub revision: u64,
}

/// Attachment-qualified capability cache.
#[derive(Debug, Default)]
pub struct CapabilityCache {
    entries: BTreeMap<DeviceKey, DeviceCapabilities>,
}

impl CapabilityCache {
    /// Returns learned capabilities or an all-unknown default.
    pub fn get(&self, key: DeviceKey) -> DeviceCapabilities {
        self.entries.get(&key).cloned().unwrap_or_default()
    }

    /// Produces the shortest safe object-list attempt sequence.
    pub fn object_list_attempts(&self, key: DeviceKey) -> Vec<ObjectListAttempt> {
        match self.entries.get(&key).and_then(|entry| entry.object_list) {
            Some(ObjectListStrategy::RpmWholeArray) => vec![
                ObjectListAttempt::RpmWholeArray,
                ObjectListAttempt::RpWholeArray,
                ObjectListAttempt::IndexedCount,
            ],
            Some(ObjectListStrategy::RpWholeArray) => vec![
                ObjectListAttempt::RpWholeArray,
                ObjectListAttempt::IndexedCount,
            ],
            Some(ObjectListStrategy::Indexed) => vec![ObjectListAttempt::IndexedCount],
            None if self.get(key).rpm == Support::Unsupported => vec![
                ObjectListAttempt::RpWholeArray,
                ObjectListAttempt::IndexedCount,
            ],
            None => vec![
                ObjectListAttempt::RpmWholeArray,
                ObjectListAttempt::RpWholeArray,
                ObjectListAttempt::IndexedCount,
            ],
        }
    }

    /// Records a successful RPM and monotonically raises its safe batch size.
    pub fn record_rpm_success(&mut self, key: DeviceKey, properties: usize) {
        let entry = self.entries.entry(key).or_default();
        let safe = entry.safe_rpm_properties.unwrap_or(0).max(properties);
        if entry.rpm != Support::Supported || entry.safe_rpm_properties != Some(safe) {
            entry.rpm = Support::Supported;
            entry.safe_rpm_properties = Some(safe);
            entry.revision += 1;
        }
    }

    /// Records a definitive RPM rejection.
    pub fn record_rpm_unsupported(&mut self, key: DeviceKey) {
        let entry = self.entries.entry(key).or_default();
        if entry.rpm != Support::Unsupported {
            entry.rpm = Support::Unsupported;
            entry.revision += 1;
        }
    }

    /// Records the successful object-list fallback strategy.
    pub fn record_object_list_success(&mut self, key: DeviceKey, strategy: ObjectListStrategy) {
        let entry = self.entries.entry(key).or_default();
        if entry.object_list != Some(strategy) {
            entry.object_list = Some(strategy);
            if strategy == ObjectListStrategy::RpmWholeArray {
                entry.rpm = Support::Supported;
            }
            entry.revision += 1;
        }
    }

    /// Removes all capabilities owned by an attachment.
    pub fn remove_attachment(&mut self, attachment_id: crate::AttachmentId) {
        self.entries
            .retain(|key, _| key.attachment_id != attachment_id);
    }

    /// Number of cached devices.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::{AttachmentId, DeviceKey};

    use super::{CapabilityCache, ObjectListAttempt, ObjectListStrategy, Support};

    fn key(attachment: u128) -> DeviceKey {
        DeviceKey {
            attachment_id: AttachmentId::from(attachment),
            device_instance: 100,
        }
    }

    #[test]
    fn learning_shortens_bounded_fallback_without_hiding_recovery() {
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.object_list_attempts(key(1)).len(), 3);
        cache.record_rpm_unsupported(key(1));
        assert_eq!(cache.get(key(1)).rpm, Support::Unsupported);
        assert_eq!(
            cache.object_list_attempts(key(1)),
            vec![
                ObjectListAttempt::RpWholeArray,
                ObjectListAttempt::IndexedCount
            ]
        );
        cache.record_object_list_success(key(1), ObjectListStrategy::Indexed);
        assert_eq!(
            cache.object_list_attempts(key(1)),
            vec![ObjectListAttempt::IndexedCount]
        );
    }

    #[test]
    fn safe_size_is_monotonic_and_attachment_owned() {
        let mut cache = CapabilityCache::default();
        cache.record_rpm_success(key(1), 8);
        cache.record_rpm_success(key(1), 4);
        cache.record_rpm_success(key(2), 16);
        assert_eq!(cache.get(key(1)).safe_rpm_properties, Some(8));
        assert_eq!(cache.get(key(1)).revision, 1);
        cache.remove_attachment(AttachmentId::from(1));
        assert_eq!(cache.get(key(1)).rpm, Support::Unknown);
        assert_eq!(cache.len(), 1);
    }
}
