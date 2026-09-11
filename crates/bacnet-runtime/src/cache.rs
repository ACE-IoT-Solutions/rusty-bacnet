use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::{AttachmentId, DeviceKey, PropertyRead};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ValueKey {
    device: DeviceKey,
    object_type: u16,
    object_instance: u32,
    property_id: u32,
    array_index: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CachedValue {
    pub(crate) raw_value: Vec<u8>,
    pub(crate) observed_at: Instant,
}

/// Attachment-owned raw observation cache used only under explicit freshness policy.
#[derive(Debug, Default)]
pub(crate) struct ValueCache {
    values: BTreeMap<ValueKey, CachedValue>,
}

impl ValueCache {
    pub(crate) fn insert(
        &mut self,
        device: DeviceKey,
        read: &PropertyRead,
        raw_value: Vec<u8>,
        observed_at: Instant,
    ) {
        self.values.insert(
            key(device, read),
            CachedValue {
                raw_value,
                observed_at,
            },
        );
    }

    pub(crate) fn get_fresh(
        &self,
        device: DeviceKey,
        read: &PropertyRead,
        max_age: Duration,
        now: Instant,
    ) -> Option<CachedValue> {
        let value = self.values.get(&key(device, read))?;
        (now.saturating_duration_since(value.observed_at) <= max_age).then(|| value.clone())
    }

    pub(crate) fn remove(&mut self, device: DeviceKey, read: &PropertyRead) {
        self.values.remove(&key(device, read));
    }

    pub(crate) fn remove_attachment(&mut self, attachment_id: AttachmentId) {
        self.values
            .retain(|key, _| key.device.attachment_id != attachment_id);
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }
}

fn key(device: DeviceKey, read: &PropertyRead) -> ValueKey {
    ValueKey {
        device,
        object_type: read.object_type,
        object_instance: read.object_instance,
        property_id: read.property_id,
        array_index: read.array_index,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{AttachmentId, DeviceKey, PropertyRead};

    use super::ValueCache;

    #[test]
    fn freshness_is_explicit_and_attachment_removal_clears_ownership() {
        let now = std::time::Instant::now();
        let device = DeviceKey {
            attachment_id: AttachmentId::from(1),
            device_instance: 10,
        };
        let read = PropertyRead {
            input_index: 99,
            object_type: 128,
            object_instance: 7,
            property_id: 512,
            array_index: Some(0),
            value_category: "unknown".to_owned(),
        };
        let mut cache = ValueCache::default();
        cache.insert(device, &read, vec![0x91, 0x01], now);
        assert!(cache
            .get_fresh(device, &read, Duration::from_secs(1), now)
            .is_some());
        assert!(cache
            .get_fresh(
                device,
                &read,
                Duration::from_millis(1),
                now + Duration::from_secs(1)
            )
            .is_none());
        cache.insert(device, &read, vec![0x91, 0x02], now);
        cache.remove(device, &read);
        assert!(cache
            .get_fresh(device, &read, Duration::from_secs(1), now)
            .is_none());
        cache.remove_attachment(device.attachment_id);
        assert_eq!(cache.len(), 0);
    }
}
