//! Admission of confirmed, single-object COV notifications for one client run.

use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use std::num::NonZeroUsize;
use std::sync::Weak;

#[derive(Clone, Eq, PartialEq, Hash)]
struct Expectation {
    source: Station,
    process: u32,
    object: ObjectIdentifier,
    initiator: Option<ObjectIdentifier>,
}

pub(super) struct Registry {
    limit: NonZeroUsize,
    entries: SyncMutex<HashMap<Expectation, Arc<()>>>,
}

impl Registry {
    pub(super) fn new(limit: NonZeroUsize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            entries: SyncMutex::new(HashMap::new()),
        })
    }

    pub(super) fn allows(&self, event: &COVEvent, local_network: Option<u16>) -> bool {
        let COVEvent::Object(received) = event else {
            return false;
        };
        let n = &received.notification;
        let entries = self.entries.lock();
        let matches = |source: Station| {
            let mut key = Expectation {
                source,
                process: n.subscriber_process_identifier,
                object: n.monitored_object_identifier,
                initiator: Some(n.initiating_device_identifier),
            };
            if entries.contains_key(&key) {
                return true;
            }
            key.initiator = None;
            entries.contains_key(&key)
        };
        match (received.source_network, received.source_address.as_deref()) {
            (None, None) => {
                matches(Station::local(&received.source_mac))
                    || entries.keys().any(|key| {
                        // A remote target can become local after the expectation was
                        // registered. Its subscription then travels without a router.
                        key.process == n.subscriber_process_identifier
                            && key.object == n.monitored_object_identifier
                            && key
                                .initiator
                                .is_none_or(|device| device == n.initiating_device_identifier)
                            && key.source.is_source_of(event, local_network)
                    })
            }
            (Some(network), Some(mac)) => {
                matches(Station::routed(&received.source_mac, network, mac))
                    || matches(Station::remote(network, mac))
            }
            _ => false,
        }
    }
}

/// Holds one local expectation for inbound confirmed COV. Closing or dropping
/// the lease withdraws it without sending a BACnet request.
#[must_use = "keep this lease alive to admit matching confirmed COV notifications"]
pub struct ConfirmedCovLease {
    registry: Weak<Registry>,
    key: Expectation,
    token: Arc<()>,
}

impl ConfirmedCovLease {
    /// Withdraw this expectation. Repeated calls are harmless, even if the
    /// same expectation was registered again.
    pub fn close(&self) {
        if let Some(registry) = self.registry.upgrade() {
            let mut entries = registry.entries.lock();
            if entries
                .get(&self.key)
                .is_some_and(|token| Arc::ptr_eq(token, &self.token))
            {
                entries.remove(&self.key);
            }
        }
    }
}

impl Drop for ConfirmedCovLease {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Admit confirmed notifications from this station for a process and
    /// monitored object, optionally restricted to the initiating device.
    /// Register before sending SubscribeCOV when strict admission is enabled.
    /// A notification with a routed source also has to match any specified router.
    pub fn expect_confirmed_cov(
        &self,
        source: Station,
        process: u32,
        object: ObjectIdentifier,
        initiating_device_instance: Option<u32>,
    ) -> Result<ConfirmedCovLease, Error> {
        let registry = self.cov_registry.as_ref().ok_or_else(|| {
            Error::Encoding("confirmed COV subscription registry is disabled".into())
        })?;
        let initiator = initiating_device_instance
            .map(|id| ObjectIdentifier::new_addressable(ObjectType::DEVICE, id))
            .transpose()?;
        let key = Expectation {
            source,
            process,
            object,
            initiator,
        };
        let mut entries = registry.entries.lock();
        if entries.contains_key(&key) {
            return Err(Error::Encoding(
                "confirmed COV expectation already registered".into(),
            ));
        }
        if entries.len() == registry.limit.get() {
            return Err(Error::Encoding(
                "confirmed COV subscription registry is full".into(),
            ));
        }
        let token = Arc::new(());
        entries.insert(key.clone(), Arc::clone(&token));
        Ok(ConfirmedCovLease {
            registry: Arc::downgrade(registry),
            key,
            token,
        })
    }
}
