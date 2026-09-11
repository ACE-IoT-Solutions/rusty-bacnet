use super::*;

pub(super) type ScanRead = (usize, u16, u32, u32, Option<u32>, String);

/// One typed property read submitted as part of a native batch.
#[pyclass(name = "RuntimeRead", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeRead {
    pub(super) input_index: usize,
    pub(super) attachment_id: u128,
    pub(super) device_instance: u32,
    pub(super) object_type: u16,
    pub(super) object_instance: u32,
    pub(super) property_id: u32,
    pub(super) array_index: Option<u32>,
    pub(super) value_category: String,
    pub(super) max_age_ms: Option<u64>,
}

#[pymethods]
impl PyRuntimeRead {
    #[new]
    #[pyo3(signature = (input_index, attachment_id, device_instance, object_type, object_instance, property_id, value_category, array_index=None, max_age_ms=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        input_index: usize,
        attachment_id: u128,
        device_instance: u32,
        object_type: u16,
        object_instance: u32,
        property_id: u32,
        value_category: String,
        array_index: Option<u32>,
        max_age_ms: Option<u64>,
    ) -> Self {
        Self {
            input_index,
            attachment_id,
            device_instance,
            object_type,
            object_instance,
            property_id,
            array_index,
            value_category,
            max_age_ms,
        }
    }
}

/// One typed authorized property write submitted as part of a native batch.
#[pyclass(name = "RuntimeWrite", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeWrite {
    pub(super) input_index: usize,
    pub(super) attachment_id: u128,
    pub(super) device_instance: u32,
    pub(super) object_type: u16,
    pub(super) object_instance: u32,
    pub(super) property_id: u32,
    pub(super) array_index: Option<u32>,
    pub(super) raw_value: Vec<u8>,
    pub(super) bacnet_priority: Option<u8>,
    pub(super) authorization_id: String,
    pub(super) verify_readback: bool,
}

#[pymethods]
impl PyRuntimeWrite {
    #[new]
    #[pyo3(signature = (input_index, attachment_id, device_instance, object_type, object_instance, property_id, value, authorization_id, array_index=None, bacnet_priority=None, verify_readback=false))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        input_index: usize,
        attachment_id: u128,
        device_instance: u32,
        object_type: u16,
        object_instance: u32,
        property_id: u32,
        value: PyPropertyValue,
        authorization_id: String,
        array_index: Option<u32>,
        bacnet_priority: Option<u8>,
        verify_readback: bool,
    ) -> PyResult<Self> {
        let mut raw_value = BytesMut::new();
        encode_property_value(&mut raw_value, value.to_rust())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self {
            input_index,
            attachment_id,
            device_instance,
            object_type,
            object_instance,
            property_id,
            array_index,
            raw_value: raw_value.to_vec(),
            bacnet_priority,
            authorization_id,
            verify_readback,
        })
    }
}

/// Ordered result of one native read or write item.
#[pyclass(name = "RuntimeBatchOutcome", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeBatchOutcome {
    #[pyo3(get)]
    pub(super) input_index: usize,
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) device_instance: u32,
    #[pyo3(get)]
    pub(super) value: Option<PyPropertyValue>,
    #[pyo3(get)]
    pub(super) raw_value: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) source: Option<String>,
    #[pyo3(get)]
    pub(super) authorization_id: Option<String>,
    #[pyo3(get)]
    pub(super) written: Option<bool>,
    #[pyo3(get)]
    pub(super) error_code: Option<String>,
    #[pyo3(get)]
    pub(super) retryable: bool,
}

/// Transactional result of applying a complete COV observation plan.
#[pyclass(name = "RuntimeObservationReport", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeObservationReport {
    #[pyo3(get)]
    pub(super) revision: u64,
    #[pyo3(get)]
    pub(super) added: usize,
    #[pyo3(get)]
    pub(super) updated: usize,
    #[pyo3(get)]
    pub(super) removed: usize,
    #[pyo3(get)]
    pub(super) idempotent: bool,
}

/// Result of requesting cancellation for one native operation identity.
#[pyclass(name = "RuntimeCancelReport", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeCancelReport {
    #[pyo3(get)]
    pub(super) operation_id: u64,
    #[pyo3(get)]
    pub(super) queued: bool,
    #[pyo3(get)]
    pub(super) in_flight: bool,
    #[pyo3(get)]
    pub(super) found: bool,
}

/// Cancellable native read operation whose ID is available before result wait.
#[pyclass(name = "RuntimeReadOperation")]
pub struct PyRuntimeReadOperation {
    #[pyo3(get)]
    pub(super) operation_id: u64,
    pub(super) inner: Arc<tokio::sync::Mutex<Option<ReadBatchOperation>>>,
    pub(super) crossings: Arc<CrossingCounters>,
}

#[pymethods]
impl PyRuntimeReadOperation {
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let operation = inner
                .lock()
                .await
                .take()
                .ok_or_else(|| PyRuntimeError::new_err("operation result already awaited"))?;
            let outcomes = operation.result().await.map_err(runtime_error)?;
            crossings
                .output_items
                .fetch_add(outcomes.len() as u64, Ordering::Relaxed);
            Ok(outcomes
                .into_iter()
                .map(read_outcome_to_py)
                .collect::<Vec<_>>())
        })
    }
}

/// Cancellable native write operation whose ID is available before result wait.
#[pyclass(name = "RuntimeWriteOperation")]
pub struct PyRuntimeWriteOperation {
    #[pyo3(get)]
    pub(super) operation_id: u64,
    pub(super) inner: Arc<tokio::sync::Mutex<Option<WriteBatchOperation>>>,
    pub(super) crossings: Arc<CrossingCounters>,
}

#[pymethods]
impl PyRuntimeWriteOperation {
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let crossings = Arc::clone(&self.crossings);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let operation = inner
                .lock()
                .await
                .take()
                .ok_or_else(|| PyRuntimeError::new_err("operation result already awaited"))?;
            let outcomes = operation.result().await.map_err(runtime_error)?;
            crossings
                .output_items
                .fetch_add(outcomes.len() as u64, Ordering::Relaxed);
            Ok(outcomes
                .into_iter()
                .map(write_outcome_to_py)
                .collect::<Vec<_>>())
        })
    }
}

/// One desired managed COV subscription in a complete observation plan.
#[pyclass(name = "RuntimeObservation", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeObservation {
    pub(super) attachment_id: u128,
    pub(super) device_instance: u32,
    pub(super) object_type: u16,
    pub(super) object_instance: u32,
    pub(super) subscriber_process_id: u32,
    pub(super) confirmed: bool,
    pub(super) lifetime_seconds: u32,
    pub(super) renewal_margin_seconds: u64,
    pub(super) suppress_poll_when_fresh: bool,
}

#[pymethods]
impl PyRuntimeObservation {
    #[new]
    #[pyo3(signature = (attachment_id, device_instance, object_type, object_instance, subscriber_process_id, lifetime_seconds, confirmed=false, renewal_margin_seconds=30, suppress_poll_when_fresh=false))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        attachment_id: u128,
        device_instance: u32,
        object_type: u16,
        object_instance: u32,
        subscriber_process_id: u32,
        lifetime_seconds: u32,
        confirmed: bool,
        renewal_margin_seconds: u64,
        suppress_poll_when_fresh: bool,
    ) -> Self {
        Self {
            attachment_id,
            device_instance,
            object_type,
            object_instance,
            subscriber_process_id,
            confirmed,
            lifetime_seconds,
            renewal_margin_seconds,
            suppress_poll_when_fresh,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct CrossingCounters {
    pub(super) calls: AtomicU64,
    pub(super) input_items: AtomicU64,
    pub(super) output_items: AtomicU64,
}
