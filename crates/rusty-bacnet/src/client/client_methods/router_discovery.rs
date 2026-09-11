use super::super::*;
use crate::types::PyRouterInfo;

#[pymethods]
impl BACnetClient {
    /// Broadcast Who-Is-Router-To-Network and collect router announcements.
    #[pyo3(signature = (network=None, observation_window_ms=500))]
    fn who_is_router_to_network<'py>(
        &self,
        py: Python<'py>,
        network: Option<u16>,
        observation_window_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let client = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let routers = client
                .who_is_router_to_network(
                    network,
                    std::time::Duration::from_millis(observation_window_ms),
                )
                .await
                .map_err(to_py_err)?;
            Ok(routers
                .into_iter()
                .map(PyRouterInfo::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Return the partial or complete snapshot from the latest router query.
    fn router_snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let client = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            Ok(client
                .router_snapshot()
                .await
                .into_iter()
                .map(PyRouterInfo::from_rust)
                .collect::<Vec<_>>())
        })
    }
}
