//! Bounded Python bridge for the native narrowing-only BVLL policy contract.

use super::*;
use bacnet_transport::bip::{BvllPolicy, BvllPolicyContext, BvllPolicyVerdict};
use bacnet_types::enums::BvlcResultCode;
use pyo3::exceptions::PyRuntimeError;
use pyo3::types::PyTuple;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

/// Bounded Python projection of an inbound BVLL datagram.
#[pyclass(name = "BvllPolicyContext", frozen)]
pub struct PyBvllPolicyContext {
    #[pyo3(get)]
    function: u8,
    #[pyo3(get)]
    source_ip: String,
    #[pyo3(get)]
    source_port: u16,
    #[pyo3(get)]
    payload_len: usize,
    payload_prefix: Vec<u8>,
}

#[pymethods]
impl PyBvllPolicyContext {
    #[getter]
    fn payload_prefix<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.payload_prefix)
    }
}

/// Object-form policy verdict. A policy cannot force native admission.
#[derive(Clone)]
#[pyclass(name = "BvllPolicyVerdict", frozen, skip_from_py_object)]
pub struct PyBvllPolicyVerdict {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    result_code: Option<u16>,
}

#[pymethods]
impl PyBvllPolicyVerdict {
    #[staticmethod]
    fn continue_native() -> Self {
        Self {
            kind: "continue".into(),
            result_code: None,
        }
    }

    #[staticmethod]
    fn drop() -> Self {
        Self {
            kind: "drop".into(),
            result_code: None,
        }
    }

    #[staticmethod]
    fn reject(result_code: u16) -> Self {
        Self {
            kind: "reject".into(),
            result_code: Some(result_code),
        }
    }
}

#[derive(Default)]
struct PolicyMetrics {
    evaluated: AtomicU64,
    continued: AtomicU64,
    dropped: AtomicU64,
    rejected: AtomicU64,
    errors: AtomicU64,
    timeouts: AtomicU64,
    overloads: AtomicU64,
    circuit_open_drops: AtomicU64,
}

/// Point-in-time Python worker counters.
#[derive(Clone)]
#[pyclass(name = "BvllPolicyCounters", frozen, skip_from_py_object)]
pub struct PyBvllPolicyCounters {
    #[pyo3(get)]
    evaluated: u64,
    #[pyo3(get)]
    continued: u64,
    #[pyo3(get)]
    dropped: u64,
    #[pyo3(get)]
    rejected: u64,
    #[pyo3(get)]
    errors: u64,
    #[pyo3(get)]
    timeouts: u64,
    #[pyo3(get)]
    overloads: u64,
    #[pyo3(get)]
    circuit_open_drops: u64,
}

enum WorkerResult {
    Verdict(BvllPolicyVerdict),
    Error,
}

struct PolicyRequest {
    context: BvllPolicyContext,
    response: mpsc::Sender<WorkerResult>,
}

struct BridgeInner {
    sender: SyncSender<PolicyRequest>,
    timeout: Duration,
    stopped: AtomicBool,
    metrics: Arc<PolicyMetrics>,
    failure_threshold: usize,
    cooldown: Duration,
    consecutive_failures: AtomicUsize,
    circuit_open_until: StdMutex<Option<Instant>>,
}

/// Cloneable bridge that never invokes Python on a transport/state-locking
/// thread. Queue overload, callback errors, invalid results, and timeouts all
/// fail closed.
#[derive(Clone)]
pub(crate) struct PythonBvllPolicyBridge {
    inner: Arc<BridgeInner>,
}

impl PythonBvllPolicyBridge {
    pub(crate) fn new(
        py: Python<'_>,
        callable: Py<PyAny>,
        timeout_ms: u64,
        queue_capacity: usize,
        failure_threshold: usize,
        cooldown_ms: u64,
    ) -> PyResult<Self> {
        if timeout_ms == 0 || queue_capacity == 0 || failure_threshold == 0 || cooldown_ms == 0 {
            return Err(PyValueError::new_err(
                "BVLL policy timeout, queue capacity, failure threshold, and cooldown must be nonzero",
            ));
        }
        if !callable.bind(py).is_callable() {
            return Err(PyValueError::new_err("BVLL policy must be callable"));
        }
        let inspect = py.import("inspect")?;
        if inspect
            .call_method1("iscoroutinefunction", (callable.bind(py),))?
            .is_truthy()?
        {
            return Err(PyValueError::new_err("BVLL policy must be synchronous"));
        }

        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let metrics = Arc::new(PolicyMetrics::default());
        let worker_metrics = Arc::clone(&metrics);
        std::thread::Builder::new()
            .name("rusty-bacnet-bvll-policy".into())
            .spawn(move || policy_worker(receiver, callable, worker_metrics))
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(BridgeInner {
                sender,
                timeout: Duration::from_millis(timeout_ms),
                stopped: AtomicBool::new(false),
                metrics,
                failure_threshold,
                cooldown: Duration::from_millis(cooldown_ms),
                consecutive_failures: AtomicUsize::new(0),
                circuit_open_until: StdMutex::new(None),
            }),
        })
    }

    pub(crate) fn counters(&self) -> PyBvllPolicyCounters {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let metrics = &self.inner.metrics;
        PyBvllPolicyCounters {
            evaluated: get(&metrics.evaluated),
            continued: get(&metrics.continued),
            dropped: get(&metrics.dropped),
            rejected: get(&metrics.rejected),
            errors: get(&metrics.errors),
            timeouts: get(&metrics.timeouts),
            overloads: get(&metrics.overloads),
            circuit_open_drops: get(&metrics.circuit_open_drops),
        }
    }

    fn circuit_is_open(&self) -> bool {
        let mut until = self
            .inner
            .circuit_open_until
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if until.is_some_and(|deadline| deadline > Instant::now()) {
            true
        } else {
            *until = None;
            false
        }
    }

    fn record_failure(&self) {
        let failures = self
            .inner
            .consecutive_failures
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if failures >= self.inner.failure_threshold {
            self.inner.consecutive_failures.store(0, Ordering::Relaxed);
            *self
                .inner
                .circuit_open_until
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) =
                Some(Instant::now() + self.inner.cooldown);
        }
    }
}

impl BvllPolicy for PythonBvllPolicyBridge {
    fn evaluate(&self, context: &BvllPolicyContext) -> BvllPolicyVerdict {
        let metrics = &self.inner.metrics;
        metrics.evaluated.fetch_add(1, Ordering::Relaxed);
        if self.inner.stopped.load(Ordering::Acquire) || self.circuit_is_open() {
            metrics.circuit_open_drops.fetch_add(1, Ordering::Relaxed);
            metrics.dropped.fetch_add(1, Ordering::Relaxed);
            return BvllPolicyVerdict::Drop;
        }
        let (tx, rx) = mpsc::channel();
        let request = PolicyRequest {
            context: context.clone(),
            response: tx,
        };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.inner.sender.try_send(request)
        {
            metrics.overloads.fetch_add(1, Ordering::Relaxed);
            metrics.dropped.fetch_add(1, Ordering::Relaxed);
            self.record_failure();
            return BvllPolicyVerdict::Drop;
        }
        match rx.recv_timeout(self.inner.timeout) {
            Ok(WorkerResult::Verdict(verdict)) => {
                self.inner.consecutive_failures.store(0, Ordering::Relaxed);
                match verdict {
                    BvllPolicyVerdict::Continue => &metrics.continued,
                    BvllPolicyVerdict::Drop => &metrics.dropped,
                    BvllPolicyVerdict::Reject(_) => &metrics.rejected,
                }
                .fetch_add(1, Ordering::Relaxed);
                verdict
            }
            Ok(WorkerResult::Error) => {
                metrics.errors.fetch_add(1, Ordering::Relaxed);
                metrics.dropped.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                BvllPolicyVerdict::Drop
            }
            Err(_) => {
                metrics.timeouts.fetch_add(1, Ordering::Relaxed);
                metrics.dropped.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                BvllPolicyVerdict::Drop
            }
        }
    }
}

fn policy_worker(
    receiver: Receiver<PolicyRequest>,
    callable: Py<PyAny>,
    _metrics: Arc<PolicyMetrics>,
) {
    while let Ok(request) = receiver.recv() {
        let result = Python::attach(|py| invoke_policy(py, &callable, request.context));
        let _ = request.response.send(result);
    }
}

fn invoke_policy(py: Python<'_>, callable: &Py<PyAny>, context: BvllPolicyContext) -> WorkerResult {
    let py_context = match Py::new(
        py,
        PyBvllPolicyContext {
            function: context.function.to_raw(),
            source_ip: Ipv4Addr::from(context.source_ip).to_string(),
            source_port: context.source_port,
            payload_len: context.payload_len,
            payload_prefix: context.payload_prefix,
        },
    ) {
        Ok(context) => context,
        Err(_) => return WorkerResult::Error,
    };
    let value = match callable.call1(py, (py_context,)) {
        Ok(value) => value,
        Err(_) => return WorkerResult::Error,
    };
    parse_verdict(value.bind(py))
}

fn parse_verdict(value: &Bound<'_, PyAny>) -> WorkerResult {
    if let Ok(kind) = value.extract::<String>() {
        return verdict_parts(&kind, None);
    }
    if let Ok(tuple) = value.cast::<PyTuple>() {
        if tuple.len() == 2 {
            if let (Ok(kind), Ok(code)) = (
                tuple.get_item(0).and_then(|item| item.extract::<String>()),
                tuple.get_item(1).and_then(|item| item.extract::<u16>()),
            ) {
                return verdict_parts(&kind, Some(code));
            }
        }
    }
    if let Ok(verdict) = value.extract::<PyRef<'_, PyBvllPolicyVerdict>>() {
        return verdict_parts(&verdict.kind, verdict.result_code);
    }
    WorkerResult::Error
}

fn verdict_parts(kind: &str, code: Option<u16>) -> WorkerResult {
    match (kind, code) {
        ("continue", None) => WorkerResult::Verdict(BvllPolicyVerdict::Continue),
        ("drop", None) => WorkerResult::Verdict(BvllPolicyVerdict::Drop),
        ("reject", Some(code)) => {
            WorkerResult::Verdict(BvllPolicyVerdict::Reject(BvlcResultCode::from_raw(code)))
        }
        _ => WorkerResult::Error,
    }
}
