use super::super::*;
use crate::types::{PyManagedCOVSubscription, PyManagedCOVTarget, PyTarget};
use std::sync::{Mutex as StdMutex, Weak};

pub(crate) type ManagedCOVRegistry =
    Arc<StdMutex<Vec<Weak<StdMutex<Option<client::ManagedCOVSubscription>>>>>>;

pub(super) async fn stop_managed_cov_subscriptions(registry: &ManagedCOVRegistry) {
    let states = registry
        .lock()
        .map(|mut registry| {
            registry
                .drain(..)
                .filter_map(|state| state.upgrade())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for state in states {
        let managed = state.lock().ok().and_then(|mut state| state.take());
        if let Some(managed) = managed {
            managed.stop().await;
        }
    }
}

#[pymethods]
impl BACnetClient {
    /// Start a finite COV subscription with automatic renewal and explicit cancellation.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime, renewal_margin_ms=30000, event_channel_capacity=16))]
    #[allow(clippy::too_many_arguments)]
    fn manage_cov_subscription<'py>(
        &self,
        py: Python<'py>,
        address: PyTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: u32,
        renewal_margin_ms: u64,
        event_channel_capacity: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let managed_cov = Arc::clone(&self.managed_cov);
        let object_id = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (mac, routing) = address.into_parts()?;
            let client = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let options = client::ManagedCOVSubscriptionOptions::default()
                .with_renewal_margin(std::time::Duration::from_millis(renewal_margin_ms))
                .with_event_channel_capacity(event_channel_capacity);

            let (managed, target) = if let Some((network, address)) = routing {
                let managed = Arc::clone(&client)
                    .manage_cov_subscription_routed(
                        &mac,
                        network,
                        &address,
                        subscriber_process_identifier,
                        object_id,
                        confirmed,
                        lifetime,
                        options,
                    )
                    .await
                    .map_err(to_py_err)?;
                (
                    managed,
                    PyManagedCOVTarget::Routed {
                        router_mac: mac,
                        network,
                        address,
                    },
                )
            } else {
                let managed = Arc::clone(&client)
                    .manage_cov_subscription(
                        &mac,
                        subscriber_process_identifier,
                        object_id,
                        confirmed,
                        lifetime,
                        options,
                    )
                    .await
                    .map_err(to_py_err)?;
                (managed, PyManagedCOVTarget::Direct(mac))
            };
            let handle = PyManagedCOVSubscription::new(
                managed,
                Arc::downgrade(&client),
                target,
                subscriber_process_identifier,
                object_id,
            );
            managed_cov
                .lock()
                .map_err(|_| PyRuntimeError::new_err("managed COV registry lock poisoned"))?
                .push(Arc::downgrade(&handle.managed));
            Ok(handle)
        })
    }
}
