use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Observe the accepted active SC Hub without network I/O. Returns None
    /// while disconnected; raises RuntimeError for a stopped or non-SC client.
    /// The frozen result is historical and does not keep the client alive.
    fn sc_peer_info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let info = {
                let client = started(&inner).await?;
                match client.transport() {
                    AnyTransport::Sc(transport) => transport.hub_peer_info().await,
                    _ => {
                        return Err(PyRuntimeError::new_err(
                            "sc_peer_info requires an SC client",
                        ))
                    }
                }
            };
            // No lifecycle lease or runtime object crosses the Python return.
            Ok(info.map(crate::sc_peer::PyScPeerInfo))
        })
    }
}
