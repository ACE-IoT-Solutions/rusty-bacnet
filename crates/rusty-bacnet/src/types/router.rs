use super::*;

/// An immutable router announcement collected from I-Am-Router-To-Network.
#[pyclass(name = "RouterInfo", frozen)]
pub struct PyRouterInfo {
    inner: RouterInfo,
}

#[pymethods]
impl PyRouterInfo {
    /// Immediate responder MAC, including a B/IP UDP port when applicable.
    #[getter]
    fn mac_address<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.source_mac)
    }

    /// Human-readable B/IP address when the responder MAC is six bytes.
    #[getter]
    fn address(&self) -> Option<String> {
        let mac = &self.inner.source_mac;
        (mac.len() == 6).then(|| {
            format!(
                "{}.{}.{}.{}:{}",
                mac[0],
                mac[1],
                mac[2],
                mac[3],
                u16::from_be_bytes([mac[4], mac[5]])
            )
        })
    }

    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.inner
            .source_network
            .as_ref()
            .map(|source| source.network)
    }

    #[getter]
    fn source_address<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .source_network
            .as_ref()
            .map(|source| PyBytes::new(py, &source.mac_address))
    }

    /// Sorted, unique advertised network numbers.
    #[getter]
    fn networks(&self) -> Vec<u16> {
        self.inner.networks.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "RouterInfo(address={:?}, networks={:?})",
            self.address(),
            self.inner.networks
        )
    }
}

impl PyRouterInfo {
    pub fn from_rust(inner: RouterInfo) -> Self {
        Self { inner }
    }
}
