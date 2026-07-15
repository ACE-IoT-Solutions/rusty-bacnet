use super::*;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial;
use pyo3::exceptions::PyRuntimeError;
use std::sync::Mutex as StdMutex;

// ---------------------------------------------------------------------------
// COV Notification — read-only wrapper
// ---------------------------------------------------------------------------

/// An incoming COV notification from a server.
#[pyclass(name = "CovNotification", frozen)]
pub struct PyCovNotification {
    inner: ReceivedCOVNotification,
}

#[pymethods]
impl PyCovNotification {
    #[getter]
    fn subscriber_process_identifier(&self) -> u32 {
        self.inner.notification.subscriber_process_identifier
    }

    #[getter]
    fn initiating_device_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.notification.initiating_device_identifier)
    }

    #[getter]
    fn monitored_object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.notification.monitored_object_identifier)
    }

    #[getter]
    fn time_remaining(&self) -> u32 {
        self.inner.notification.time_remaining
    }

    #[getter]
    fn delivery(&self) -> &'static str {
        match self.inner.delivery {
            COVNotificationDelivery::Confirmed => "confirmed",
            COVNotificationDelivery::Unconfirmed => "unconfirmed",
        }
    }

    #[getter]
    fn source_mac(&self, py: Python<'_>) -> Py<PyAny> {
        PyBytes::new(py, &self.inner.source_mac).into_any().unbind()
    }

    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.inner.source_network
    }

    #[getter]
    fn source_address(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.inner
            .source_address
            .as_ref()
            .map(|mac| PyBytes::new(py, mac).into_any().unbind())
    }

    /// List of property values as dicts with `property_id`, `array_index`, `value`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for pv in &self.inner.notification.list_of_values {
            let dict = PyDict::new(py);
            dict.set_item(
                "property_id",
                PyPropertyIdentifier {
                    inner: pv.property_identifier,
                },
            )?;
            dict.set_item("array_index", pv.property_array_index)?;
            if !pv.value.is_empty() {
                match decode_application_value(&pv.value, 0) {
                    Ok((val, _)) => {
                        dict.set_item("value", PyPropertyValue::from_rust(val))?;
                    }
                    Err(_) => {
                        dict.set_item("value", PyBytes::new(py, &pv.value))?;
                    }
                }
            } else {
                dict.set_item("value", py.None())?;
            }
            list.append(dict)?;
        }
        Ok(list.into_any().unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "CovNotification(device={}, object={}, delivery={}, remaining={})",
            self.inner
                .notification
                .initiating_device_identifier
                .instance_number(),
            self.inner.notification.monitored_object_identifier,
            self.delivery(),
            self.inner.notification.time_remaining
        )
    }
}

// ---------------------------------------------------------------------------
// COV Notification async iterator
// ---------------------------------------------------------------------------

/// Async iterator yielding COV notifications from a broadcast channel.
#[pyclass(name = "CovNotificationIterator")]
pub struct PyCovNotificationIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<ReceivedCOVNotification>>>,
}

impl PyCovNotificationIterator {
    pub fn new(rx: broadcast::Receiver<ReceivedCOVNotification>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

#[derive(Clone)]
pub enum PyManagedCOVTarget {
    Direct(Vec<u8>),
    Routed {
        router_mac: Vec<u8>,
        network: u16,
        address: Vec<u8>,
    },
}

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
                    let error = crate::errors::BacnetNotificationLagError::new_err(format!(
                        "managed COV event iterator skipped {skipped} events"
                    ));
                    Python::attach(|py| {
                        let _ = error.value(py).setattr("skipped", skipped);
                    });
                    Err(error)
                }
                Err(broadcast::error::RecvError::Closed) => {
                    Err(PyStopAsyncIteration::new_err("channel closed"))
                }
            }
        })
    }
}

#[pyclass(name = "ManagedCOVSubscription")]
pub struct PyManagedCOVSubscription {
    pub(crate) managed: crate::client::ManagedCOVState,
    client: std::sync::Weak<bacnet_client::client::BACnetClient<AnyTransport<NoSerial>>>,
    target: PyManagedCOVTarget,
    subscriber_process_identifier: u32,
    monitored_object_identifier: primitives::ObjectIdentifier,
}

impl PyManagedCOVSubscription {
    pub fn new(
        managed: ManagedCOVSubscription,
        client: std::sync::Weak<bacnet_client::client::BACnetClient<AnyTransport<NoSerial>>>,
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
            managed.as_ref().is_none_or(|item| item.is_finished())
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

#[pymethods]
impl PyCovNotificationIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = self.rx.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = rx.lock().await;
            match guard.recv().await {
                Ok(notif) => Ok(PyCovNotification { inner: notif }),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let error = crate::errors::BacnetNotificationLagError::new_err(format!(
                        "COV notification iterator skipped {n} messages"
                    ));
                    Python::attach(|py| {
                        let _ = error.value(py).setattr("skipped", n);
                    });
                    Err(error)
                }
                Err(broadcast::error::RecvError::Closed) => {
                    Err(PyStopAsyncIteration::new_err("channel closed"))
                }
            }
        })
    }
}
