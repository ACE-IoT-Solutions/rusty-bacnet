use super::*;

use bacnet_client::discovery::IAmEvent;

/// One I-Am observation, including transport and BVLL provenance.
///
/// This stays separate from `DiscoveredDevice`: discovery merges by device
/// instance, while this value represents every valid packet, including copies.
#[pyclass(name = "IAmEvent", frozen)]
pub struct PyIAmEvent {
    inner: IAmEvent,
    timestamp: f64,
}

impl PyIAmEvent {
    fn from_rust(py: Python<'_>, inner: IAmEvent) -> PyResult<Self> {
        // Anchor Rust's opaque Instant in Python's monotonic clock domain once.
        let now = py
            .import("time")?
            .call_method0("monotonic")?
            .extract::<f64>()?;
        let timestamp = now - inner.timestamp.elapsed().as_secs_f64();
        Ok(Self { inner, timestamp })
    }
}

#[pymethods]
impl PyIAmEvent {
    #[getter]
    fn device_instance(&self) -> u32 {
        self.inner.device_instance
    }

    #[getter]
    fn device_id(&self) -> u32 {
        self.inner.device_instance
    }

    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.object_identifier)
    }

    #[getter]
    fn max_apdu_length(&self) -> u32 {
        self.inner.max_apdu_length
    }

    #[getter]
    fn segmentation_supported(&self) -> PySegmentation {
        PySegmentation {
            inner: self.inner.segmentation_supported,
        }
    }

    #[getter]
    fn vendor_id(&self) -> u16 {
        self.inner.vendor_id
    }

    /// Immediate UDP peer as `ip:port`; `None` on non-IPv4 links.
    #[getter]
    fn udp_source(&self) -> Option<String> {
        format_ipv4_endpoint(self.inner.udp_source_ip, self.inner.udp_source_port)
    }

    /// Raw transport source MAC bytes.
    #[getter]
    fn source_mac(&self, py: Python<'_>) -> Py<PyAny> {
        PyBytes::new(py, self.inner.source_mac.as_slice())
            .into_any()
            .unbind()
    }

    #[getter]
    fn raw_mac(&self, py: Python<'_>) -> Py<PyAny> {
        self.source_mac(py)
    }

    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.inner.source_network
    }

    #[getter]
    fn snet(&self) -> Option<u16> {
        self.inner.source_network
    }

    #[getter]
    fn source_address(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.inner
            .source_address
            .as_ref()
            .map(|address| PyBytes::new(py, address.as_slice()).into_any().unbind())
    }

    #[getter]
    fn sadr(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.source_address(py)
    }

    #[getter]
    fn bvlc_function(&self) -> Option<u8> {
        self.inner.bvlc_function
    }

    #[getter]
    fn bvll_function(&self) -> Option<u8> {
        self.inner.bvlc_function
    }

    /// Origin embedded in a Forwarded-NPDU, distinct from `udp_source`.
    #[getter]
    fn forwarded_from(&self) -> Option<String> {
        format_ipv4_endpoint(self.inner.forwarded_from_ip, self.inner.forwarded_from_port)
    }

    #[getter]
    fn timestamp(&self) -> f64 {
        self.timestamp
    }

    fn __repr__(&self) -> String {
        format!(
            "IAmEvent(device_instance={}, vendor_id={}, udp_source={:?}, bvlc_function={:?})",
            self.inner.device_instance,
            self.inner.vendor_id,
            self.udp_source(),
            self.inner.bvlc_function,
        )
    }
}

fn format_ipv4_endpoint(ip: Option<[u8; 4]>, port: Option<u16>) -> Option<String> {
    match (ip, port) {
        (Some(ip), Some(port)) => Some(format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port)),
        _ => None,
    }
}

/// Async iterator over the client's bounded I-Am broadcast channel.
#[pyclass(name = "IAmEventIterator")]
pub struct PyIAmEventIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<IAmEvent>>>,
}

impl PyIAmEventIterator {
    pub fn new(rx: broadcast::Receiver<IAmEvent>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

#[pymethods]
impl PyIAmEventIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = Arc::clone(&self.rx);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut rx = rx.lock().await;
            match rx.recv().await {
                Ok(event) => Python::attach(|py| PyIAmEvent::from_rust(py, event)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let error = crate::errors::BacnetNotificationLagError::new_err(format!(
                        "I-Am event iterator skipped {skipped} events"
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

#[cfg(test)]
mod tests {
    use super::format_ipv4_endpoint;

    #[test]
    fn endpoint_format_preserves_non_default_port() {
        assert_eq!(
            format_ipv4_endpoint(Some([192, 0, 2, 7]), Some(47_909)).as_deref(),
            Some("192.0.2.7:47909")
        );
        assert_eq!(format_ipv4_endpoint(Some([192, 0, 2, 7]), None), None);
    }
}
