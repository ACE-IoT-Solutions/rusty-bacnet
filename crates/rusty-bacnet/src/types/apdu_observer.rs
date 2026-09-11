//! Python projection of the opt-in, diagnostic-only network APDU observer.

use std::sync::{Arc, Mutex as StdMutex};

use bacnet_network::observer::{
    ApduDecode, ApduDirection, ApduObserver, ApduObserverEvent, DecodeStage,
    MAX_APDU_OBSERVER_CAPACITY,
};
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::broadcast;

pub(crate) const DEFAULT_APDU_OBSERVER_CAPACITY: usize = 64;

/// A typed diagnostic decode failure. Decode errors are events, not iterator errors.
#[pyclass(name = "ApduDecodeError", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyApduDecodeError {
    stage: &'static str,
    category: &'static str,
    message: String,
}

#[pymethods]
impl PyApduDecodeError {
    #[getter]
    fn stage(&self) -> &'static str {
        self.stage
    }

    #[getter]
    fn category(&self) -> &'static str {
        self.category
    }

    #[getter]
    fn message(&self) -> &str {
        &self.message
    }

    fn __repr__(&self) -> String {
        format!(
            "ApduDecodeError(stage={:?}, category={:?}, message={:?})",
            self.stage, self.category, self.message
        )
    }
}

/// Immutable diagnostic observation at the network-layer APDU boundary.
///
/// This API is unstable. `raw_npdu` can contain credentials, private-transfer
/// payloads, and other application data and must be handled as sensitive.
#[pyclass(name = "ApduObserverEvent", frozen)]
pub struct PyApduObserverEvent {
    direction: &'static str,
    immediate_peer: Vec<u8>,
    claimed_forwarded_origin: Option<Vec<u8>>,
    routed_network: Option<u16>,
    routed_address: Option<Vec<u8>>,
    raw_npdu: Vec<u8>,
    pdu_type: Option<&'static str>,
    service_choice: Option<u32>,
    invoke_id: Option<u8>,
    decode_error: Option<PyApduDecodeError>,
}

impl From<ApduObserverEvent> for PyApduObserverEvent {
    fn from(event: ApduObserverEvent) -> Self {
        let direction = match event.direction {
            ApduDirection::Inbound => "inbound",
            ApduDirection::Outbound => "outbound",
        };
        let (pdu_type, service_choice, invoke_id, decode_error) = match event.decode {
            ApduDecode::Decoded(summary) => (
                Some(summary.pdu_type),
                summary.service_choice,
                summary.invoke_id,
                None,
            ),
            ApduDecode::Error(error) => (
                None,
                None,
                None,
                Some(PyApduDecodeError {
                    stage: match error.stage {
                        DecodeStage::Npdu => "npdu",
                        DecodeStage::Apdu => "apdu",
                    },
                    category: error.category,
                    message: error.message,
                }),
            ),
        };
        let (routed_network, routed_address) = event
            .routed_address
            .map(|address| (Some(address.network), Some(address.mac_address.to_vec())))
            .unwrap_or((None, None));

        Self {
            direction,
            immediate_peer: event.immediate_peer.to_vec(),
            claimed_forwarded_origin: event.claimed_forwarded_origin.map(|mac| mac.to_vec()),
            routed_network,
            routed_address,
            raw_npdu: event.raw_npdu.to_vec(),
            pdu_type,
            service_choice,
            invoke_id,
            decode_error,
        }
    }
}

#[pymethods]
impl PyApduObserverEvent {
    #[getter]
    fn direction(&self) -> &'static str {
        self.direction
    }

    #[getter]
    fn immediate_peer(&self, py: Python<'_>) -> Py<PyAny> {
        PyBytes::new(py, &self.immediate_peer).into_any().unbind()
    }

    /// Origin claimed by Forwarded-NPDU framing. This value is untrusted.
    #[getter]
    fn claimed_forwarded_origin(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.claimed_forwarded_origin
            .as_ref()
            .map(|value| PyBytes::new(py, value).into_any().unbind())
    }

    #[getter]
    fn routed_network(&self) -> Option<u16> {
        self.routed_network
    }

    #[getter]
    fn routed_address(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.routed_address
            .as_ref()
            .map(|value| PyBytes::new(py, value).into_any().unbind())
    }

    #[getter]
    fn raw_npdu(&self, py: Python<'_>) -> Py<PyAny> {
        PyBytes::new(py, &self.raw_npdu).into_any().unbind()
    }

    #[getter]
    fn pdu_type(&self) -> Option<&'static str> {
        self.pdu_type
    }

    #[getter]
    fn service_choice(&self) -> Option<u32> {
        self.service_choice
    }

    #[getter]
    fn invoke_id(&self) -> Option<u8> {
        self.invoke_id
    }

    #[getter]
    fn decode_error(&self) -> Option<PyApduDecodeError> {
        self.decode_error.clone()
    }

    #[getter]
    fn decoded(&self) -> bool {
        self.decode_error.is_none()
    }

    fn __repr__(&self) -> String {
        format!(
            "ApduObserverEvent(direction={:?}, pdu_type={:?}, service_choice={:?}, decode_error={:?})",
            self.direction,
            self.pdu_type,
            self.service_choice,
            self.decode_error.as_ref().map(|error| error.stage),
        )
    }
}

/// Async iterator over one bounded observer generation.
#[pyclass(name = "ApduObserverEventIterator", frozen)]
pub struct PyApduObserverEventIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<ApduObserverEvent>>>,
}

impl PyApduObserverEventIterator {
    fn new(rx: broadcast::Receiver<ApduObserverEvent>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

#[pymethods]
impl PyApduObserverEventIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = Arc::clone(&self.rx);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut rx = rx.lock().await;
            match rx.recv().await {
                Ok(event) => Ok(PyApduObserverEvent::from(event)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let error = crate::errors::BacnetNotificationLagError::new_err(format!(
                        "APDU observer event iterator skipped {skipped} events"
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

struct ObserverGeneration {
    observer: ApduObserver,
    initial_rx: Option<broadcast::Receiver<ApduObserverEvent>>,
}

/// Python lifecycle state. Reserving the initial receiver at construction
/// removes the first-packet subscription race without crossing into Python.
pub(crate) struct PyApduObserverState {
    capacity: usize,
    generation: StdMutex<ObserverGeneration>,
}

/// Resets an observer generation if Python cancels startup after the native
/// network layer has taken a publisher clone.
pub(crate) struct PyApduObserverStartGuard {
    state: Option<Arc<PyApduObserverState>>,
    committed: bool,
}

impl PyApduObserverStartGuard {
    pub(crate) fn new(state: Option<Arc<PyApduObserverState>>) -> Self {
        Self {
            state,
            committed: false,
        }
    }

    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PyApduObserverStartGuard {
    fn drop(&mut self) {
        if !self.committed {
            if let Some(state) = self.state.as_ref() {
                state.close_and_reset();
            }
        }
    }
}

impl PyApduObserverState {
    pub(crate) fn configured(enabled: bool, capacity: usize) -> PyResult<Option<Arc<Self>>> {
        if !enabled {
            if capacity != DEFAULT_APDU_OBSERVER_CAPACITY {
                return Err(PyValueError::new_err(
                    "apdu_observer_capacity requires apdu_observer=True",
                ));
            }
            return Ok(None);
        }
        if capacity == 0 {
            return Err(PyValueError::new_err(
                "apdu_observer_capacity must be greater than zero",
            ));
        }
        if capacity > MAX_APDU_OBSERVER_CAPACITY {
            return Err(PyValueError::new_err(format!(
                "apdu_observer_capacity must be at most {MAX_APDU_OBSERVER_CAPACITY}"
            )));
        }
        let (observer, initial_rx) = ApduObserver::new(capacity);
        Ok(Some(Arc::new(Self {
            capacity,
            generation: StdMutex::new(ObserverGeneration {
                observer,
                initial_rx: Some(initial_rx),
            }),
        })))
    }

    pub(crate) fn observer(&self) -> PyResult<ApduObserver> {
        self.generation
            .lock()
            .map(|generation| generation.observer.clone())
            .map_err(|_| PyValueError::new_err("APDU observer lock is poisoned"))
    }

    pub(crate) fn events(&self) -> PyResult<PyApduObserverEventIterator> {
        let mut generation = self
            .generation
            .lock()
            .map_err(|_| PyValueError::new_err("APDU observer lock is poisoned"))?;
        let rx = match generation.initial_rx.take() {
            Some(rx) => rx,
            None => generation
                .observer
                .subscribe()
                .ok_or_else(|| PyValueError::new_err("APDU observer lifecycle is closed"))?,
        };
        Ok(PyApduObserverEventIterator::new(rx))
    }

    /// Close old iterators and reserve a receiver for the next client start.
    pub(crate) fn close_and_reset(&self) {
        if let Ok(mut generation) = self.generation.lock() {
            generation.observer.close();
            let (observer, initial_rx) = ApduObserver::new(self.capacity);
            *generation = ObserverGeneration {
                observer,
                initial_rx: Some(initial_rx),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_state_rejects_non_default_capacity() {
        assert!(PyApduObserverState::configured(false, 1).is_err());
        assert!(
            PyApduObserverState::configured(false, DEFAULT_APDU_OBSERVER_CAPACITY)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn enabled_state_validates_capacity() {
        assert!(PyApduObserverState::configured(true, 0).is_err());
        assert!(PyApduObserverState::configured(true, MAX_APDU_OBSERVER_CAPACITY + 1).is_err());
        assert!(PyApduObserverState::configured(true, 1).unwrap().is_some());
    }
}
