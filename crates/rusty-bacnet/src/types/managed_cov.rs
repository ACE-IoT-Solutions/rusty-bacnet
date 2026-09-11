use super::*;
use bacnet_client::client::{ManagedCOVSubscription, ManagedCOVSubscriptionEvent};
use bacnet_transport::any::AnyTransport;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use std::sync::{Mutex as StdMutex, Weak};

type ManagedClient = bacnet_client::client::BACnetClient<AnyTransport<crate::mstp_py::PySerial>>;

pub(crate) type PyManagedCOVState = Arc<StdMutex<Option<ManagedCOVSubscription>>>;

#[derive(Clone)]
pub(crate) enum PyManagedCOVTarget {
    Direct(Vec<u8>),
    Routed {
        router_mac: Vec<u8>,
        network: u16,
        address: Vec<u8>,
    },
}

/// Renewal, expiry, failure, or backpressure observation for a managed COV subscription.
#[pyclass(name = "ManagedCOVEvent", frozen)]
pub struct PyManagedCOVEvent {
    inner: ManagedCOVSubscriptionEvent,
}

impl PyManagedCOVEvent {
    fn from_rust(inner: ManagedCOVSubscriptionEvent) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyManagedCOVEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            ManagedCOVSubscriptionEvent::NotificationObserved { .. } => "notification_observed",
            ManagedCOVSubscriptionEvent::ImpendingExpiry { .. } => "impending_expiry",
            ManagedCOVSubscriptionEvent::Renewed { .. } => "renewed",
            ManagedCOVSubscriptionEvent::RenewalFailed { .. } => "renewal_failed",
            ManagedCOVSubscriptionEvent::NotificationLagged { .. } => "notification_lagged",
            _ => "unknown",
        }
    }

    #[getter]
    fn time_remaining(&self) -> Option<u32> {
        match self.inner {
            ManagedCOVSubscriptionEvent::NotificationObserved { time_remaining, .. }
            | ManagedCOVSubscriptionEvent::ImpendingExpiry { time_remaining } => {
                Some(time_remaining)
            }
            _ => None,
        }
    }

    #[getter]
    fn requested_lifetime(&self) -> Option<u32> {
        match self.inner {
            ManagedCOVSubscriptionEvent::Renewed {
                requested_lifetime, ..
            } => Some(requested_lifetime),
            _ => None,
        }
    }

    #[getter]
    fn renew_after_ms(&self) -> Option<u64> {
        match self.inner {
            ManagedCOVSubscriptionEvent::NotificationObserved { renew_after, .. }
            | ManagedCOVSubscriptionEvent::Renewed { renew_after, .. } => {
                Some(renew_after.as_millis().min(u128::from(u64::MAX)) as u64)
            }
            _ => None,
        }
    }

    #[getter]
    fn error(&self) -> Option<&str> {
        match &self.inner {
            ManagedCOVSubscriptionEvent::RenewalFailed { error } => Some(error),
            _ => None,
        }
    }

    #[getter]
    fn skipped(&self) -> Option<u64> {
        match self.inner {
            ManagedCOVSubscriptionEvent::NotificationLagged { skipped } => Some(skipped),
            _ => None,
        }
    }
}

/// Async iterator of managed COV lifecycle events.
#[pyclass(name = "ManagedCOVEventIterator")]
pub struct PyManagedCOVEventIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<ManagedCOVSubscriptionEvent>>>,
}

impl PyManagedCOVEventIterator {
    fn new(rx: broadcast::Receiver<ManagedCOVSubscriptionEvent>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

#[pymethods]
impl PyManagedCOVEventIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = Arc::clone(&self.rx);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut rx = rx.lock().await;
            match rx.recv().await {
                Ok(event) => Ok(PyManagedCOVEvent::from_rust(event)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    Ok(PyManagedCOVEvent::from_rust(
                        ManagedCOVSubscriptionEvent::NotificationLagged { skipped },
                    ))
                }
                Err(broadcast::error::RecvError::Closed) => {
                    Err(PyStopAsyncIteration::new_err("channel closed"))
                }
            }
        })
    }
}

/// Handle for a finite COV subscription renewed by the Rust client.
#[pyclass(name = "ManagedCOVSubscription")]
pub struct PyManagedCOVSubscription {
    pub(crate) managed: PyManagedCOVState,
    client: Weak<ManagedClient>,
    target: PyManagedCOVTarget,
    subscriber_process_identifier: u32,
    monitored_object_identifier: primitives::ObjectIdentifier,
}

impl PyManagedCOVSubscription {
    pub(crate) fn new(
        managed: ManagedCOVSubscription,
        client: Weak<ManagedClient>,
        target: PyManagedCOVTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: primitives::ObjectIdentifier,
    ) -> Self {
        Self {
            managed: Arc::new(StdMutex::new(Some(managed))),
            client,
            target,
            subscriber_process_identifier,
            monitored_object_identifier,
        }
    }
}

#[pymethods]
impl PyManagedCOVSubscription {
    #[getter]
    fn closed(&self) -> bool {
        self.managed
            .lock()
            .map_or(true, |managed| managed.is_none())
    }

    #[getter]
    fn finished(&self) -> bool {
        self.managed.lock().map_or(true, |managed| {
            managed
                .as_ref()
                .is_none_or(ManagedCOVSubscription::is_finished)
        })
    }

    #[getter]
    fn last_event(&self) -> Option<PyManagedCOVEvent> {
        self.managed.lock().ok().and_then(|managed| {
            managed
                .as_ref()
                .and_then(ManagedCOVSubscription::last_event)
                .map(PyManagedCOVEvent::from_rust)
        })
    }

    fn events(&self) -> PyResult<PyManagedCOVEventIterator> {
        let managed = self
            .managed
            .lock()
            .map_err(|_| PyRuntimeError::new_err("managed COV handle lock poisoned"))?;
        let managed = managed
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("managed COV subscription is closed"))?;
        Ok(PyManagedCOVEventIterator::new(managed.events()))
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let managed = Arc::clone(&self.managed);
        let client = self.client.clone();
        let target = self.target.clone();
        let process_id = self.subscriber_process_identifier;
        let object_id = self.monitored_object_identifier;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let item = managed
                .lock()
                .map_err(|_| PyRuntimeError::new_err("managed COV handle lock poisoned"))?
                .take();
            let Some(item) = item else {
                return Ok(());
            };
            item.stop().await;
            let Some(client) = client.upgrade() else {
                return Ok(());
            };
            match target {
                PyManagedCOVTarget::Direct(mac) => {
                    client.unsubscribe_cov(&mac, process_id, object_id).await
                }
                PyManagedCOVTarget::Routed {
                    router_mac,
                    network,
                    address,
                } => {
                    client
                        .unsubscribe_cov_routed(
                            &router_mac,
                            network,
                            &address,
                            process_id,
                            object_id,
                        )
                        .await
                }
            }
            .map_err(crate::errors::to_py_err)?;
            Ok(())
        })
    }

    fn cancel<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.close(py)
    }
}
