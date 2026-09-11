use super::*;
use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};

/// Immutable Broadcast Distribution Table entry returned by `read_bdt`.
#[derive(Clone)]
#[pyclass(name = "BdtEntry", frozen, from_py_object)]
pub struct PyBdtEntry {
    inner: BdtEntry,
}

#[pymethods]
impl PyBdtEntry {
    #[getter]
    fn ip(&self) -> String {
        Ipv4Addr::from(self.inner.ip).to_string()
    }

    #[getter]
    fn ip_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.ip)
    }

    #[getter]
    fn port(&self) -> u16 {
        self.inner.port
    }

    #[getter]
    fn broadcast_mask(&self) -> String {
        Ipv4Addr::from(self.inner.broadcast_mask).to_string()
    }

    #[getter]
    fn broadcast_mask_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.broadcast_mask)
    }

    fn __repr__(&self) -> String {
        format!(
            "BdtEntry(ip='{}', port={}, broadcast_mask='{}')",
            self.ip(),
            self.port(),
            self.broadcast_mask()
        )
    }
}

impl PyBdtEntry {
    pub(crate) fn from_rust(inner: BdtEntry) -> Self {
        Self { inner }
    }
}

/// Immutable Foreign Device Table wire entry returned by `read_fdt`.
#[derive(Clone)]
#[pyclass(name = "FdtEntry", frozen, from_py_object)]
pub struct PyFdtEntry {
    inner: FdtEntryWire,
}

#[pymethods]
impl PyFdtEntry {
    #[getter]
    fn ip(&self) -> String {
        Ipv4Addr::from(self.inner.ip).to_string()
    }

    #[getter]
    fn ip_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.ip)
    }

    #[getter]
    fn port(&self) -> u16 {
        self.inner.port
    }

    #[getter]
    fn ttl(&self) -> u16 {
        self.inner.ttl
    }

    #[getter]
    fn seconds_remaining(&self) -> u16 {
        self.inner.seconds_remaining
    }

    fn __repr__(&self) -> String {
        format!(
            "FdtEntry(ip='{}', port={}, ttl={}, seconds_remaining={})",
            self.ip(),
            self.port(),
            self.ttl(),
            self.seconds_remaining()
        )
    }
}

impl PyFdtEntry {
    pub(crate) fn from_rust(inner: FdtEntryWire) -> Self {
        Self { inner }
    }
}
