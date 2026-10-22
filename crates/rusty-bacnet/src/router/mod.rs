//! B/IP-only forwarding owner with retained native lifecycle and copied state.
mod config;
mod errors;
mod lifecycle;
mod publication;
mod records;
mod session;
#[cfg(test)]
mod tests;

use config::{PortConfig, PyBipRouterPort};
use lifecycle::Lifecycle;
use publication::Publication;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use session::NativeSession;
use std::sync::Mutex;

/// A standalone B/IP router; this first binding exposes no local application role.
///
/// Use start/close or an async context. Successful awaited close joins all
/// workers and transport cleanup. Failed close retains errors and prevents
/// restart. Cancelled waits do not own teardown; await close for its barrier.
#[pyclass(name = "BACnetRouter", module = "rusty_bacnet")]
pub(crate) struct PyBACnetRouter {
    configs: Vec<PortConfig>,
    lifecycle: Lifecycle<NativeSession>,
    event_loop: Mutex<Option<Py<PyAny>>>,
}

impl PyBACnetRouter {
    fn networks(&self) -> Vec<u16> {
        self.configs
            .iter()
            .map(|config| config.network_number)
            .collect()
    }
    fn bind_loop(&self, py: Python<'_>) -> PyResult<()> {
        let current = py.import("asyncio")?.call_method0("get_running_loop")?;
        let mut owner = self.event_loop.lock().unwrap_or_else(|e| e.into_inner());
        match owner.as_ref() {
            Some(expected) if !expected.bind(py).is(&current) => Err(PyRuntimeError::new_err(
                "router belongs to a different asyncio event loop",
            )),
            Some(_) => Ok(()),
            None => {
                *owner = Some(current.unbind());
                Ok(())
            }
        }
    }
}

#[pymethods]
impl PyBACnetRouter {
    /// Validate one to 255 immutable B/IP ports before any traffic.
    #[new]
    fn new(ports: Vec<PyRef<'_, PyBipRouterPort>>) -> PyResult<Self> {
        if ports.is_empty() || ports.len() > 255 {
            return Err(PyValueError::new_err(
                "router requires between 1 and 255 ports",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        let mut configs = Vec::with_capacity(ports.len());
        for port in ports {
            if !seen.insert(port.0.network_number) {
                return Err(PyValueError::new_err(format!(
                    "duplicate network_number {}",
                    port.0.network_number
                )));
            }
            configs.push(port.0.clone());
        }
        Ok(Self {
            configs,
            lifecycle: Lifecycle::new(),
            event_loop: Mutex::new(None),
        })
    }

    /// Start this configuration; duplicate explicit start raises RuntimeError.
    /// The returned object is an Awaitable, not necessarily a coroutine.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.bind_loop(py)?;
        crate::logging_bridge::refresh_at_start(py);
        let owner = self.lifecycle.clone();
        let config = self.configs.clone();
        let networks = self.networks();
        let publication = Publication::new(owner.clone());
        let hook = publication.clone();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                let generation = owner
                    .start(false, move || NativeSession::new(config))
                    .await
                    .map_err(|e| errors::start_error(e, &networks))?;
                publication.arm(generation);
                crate::py_async::attach(|py| Ok(py.None()))
            },
            hook,
        )
    }

    /// Await retained cleanup. Repeated failures remain failures and block restart.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.bind_loop(py)?;
        let owner = self.lifecycle.clone();
        let networks = self.networks();
        crate::py_async::future_into_py(py, async move {
            owner
                .close()
                .await
                .map_err(|e| errors::close_error(&e, &networks))?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Enter the owner, reusing its running generation if already started.
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let (owner, config, networks) = {
            let this = slf.borrow();
            this.bind_loop(py)?;
            (
                this.lifecycle.clone(),
                this.configs.clone(),
                this.networks(),
            )
        };
        crate::logging_bridge::refresh_at_start(py);
        let publication = Publication::new(owner.clone());
        let hook = publication.clone();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                let generation = owner
                    .start(true, move || NativeSession::new(config))
                    .await
                    .map_err(|e| errors::start_error(e, &networks))?;
                publication.arm(generation);
                Ok(self_ref)
            },
            hook,
        )
    }

    /// Await cleanup while preserving any context-body exception as primary.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        exc_type: Option<Bound<'py, PyAny>>,
        exc_value: Option<Bound<'py, PyAny>>,
        traceback: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.bind_loop(py)?;
        let body_error = exc_value.is_some_and(|value| !value.is_none());
        let _ = (exc_type, traceback);
        let owner = self.lifecycle.clone();
        let networks = self.networks();
        crate::py_async::future_into_py(py, async move {
            errors::context_outcome(owner.close().await, body_error, &networks)?;
            crate::py_async::attach(|py| Ok(py.None()))
        })
    }

    /// Copy actual post-start addresses in configured input order.
    fn ports<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.bind_loop(py)?;
        let owner = self.lifecycle.clone();
        crate::py_async::future_into_py(py, async move {
            owner
                .inspect(NativeSession::ports)
                .await
                .ok_or_else(|| PyRuntimeError::new_err("router not started or already closing"))
        })
    }

    /// Copy direct and learned routes, evaluating reachability at capture time.
    fn routes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.bind_loop(py)?;
        let owner = self.lifecycle.clone();
        crate::py_async::future_into_py(py, async move {
            owner
                .inspect(NativeSession::routes)
                .await
                .ok_or_else(|| PyRuntimeError::new_err("router not started or already closing"))
        })
    }
}

impl Drop for PyBACnetRouter {
    fn drop(&mut self) {
        self.lifecycle.dispose();
        if self
            .event_loop
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            let owner = self.lifecycle.clone();
            crate::py_async::spawn_cleanup(async move {
                let _ = owner.close().await;
            });
        }
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyBACnetRouter>()?;
    module.add_class::<PyBipRouterPort>()?;
    module.add_class::<records::PyRouterPortInfo>()?;
    module.add_class::<records::PyRouterRoute>()?;
    module.add_class::<records::PyRouterCleanupFailure>()?;
    module.add(
        "BacnetRouterCloseError",
        module.py().get_type::<errors::BacnetRouterCloseError>(),
    )?;
    Ok(())
}
