//! Synchronous owned offline reports, with explicit byte and path boundaries.

use crate::sc_peer::PyScCertificateMetadata;
use bacnet_transport::sc_tls::inspection::{
    self, ScExtensionInspection, ScInputErrorKind, ScInputRole, ScInspectedCertificate,
    ScInspectionError, ScInspectionFinding, ScKeyFilePermissions, ScPemInspection, MAX_INPUT_BYTES,
};
use pyo3::{
    exceptions::{PyOSError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyBool, PyByteArray, PyBytes, PyInt, PyString, PyTuple},
};
use std::{borrow::Cow, path::PathBuf};

#[pyclass(name = "ScPemInspection", module = "rusty_bacnet", frozen)]
struct PyScPemInspection(ScPemInspection);
#[pymethods]
impl PyScPemInspection {
    #[getter]
    fn inspected_at_unix(&self) -> i64 {
        self.0.inspected_at_unix
    }
    #[getter]
    fn peer_trust_supplied(&self) -> bool {
        self.0.peer_trust_supplied
    }
    #[getter]
    fn key_consistency(&self) -> &'static str {
        self.0.key_consistency.as_str()
    }
    #[getter]
    fn key_file_permissions(&self) -> Option<PyScKeyFilePermissions> {
        self.0.key_file_permissions.map(PyScKeyFilePermissions)
    }
    #[getter]
    fn certificates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.0
                .certificates
                .iter()
                .map(|v| Py::new(py, PyScInspectedCertificate(v.clone())))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "ScPemInspection(inspected_at_unix={}, certificate_count={}, key_consistency='{}')",
            self.inspected_at_unix(),
            self.0.certificates.len(),
            self.key_consistency()
        )
    }
}

#[pyclass(name = "ScInspectedCertificate", module = "rusty_bacnet", frozen)]
struct PyScInspectedCertificate(ScInspectedCertificate);
#[pymethods]
impl PyScInspectedCertificate {
    #[getter]
    fn role(&self) -> &'static str {
        self.0.role.as_str()
    }
    #[getter]
    fn index(&self) -> usize {
        self.0.index
    }
    #[getter]
    fn sha256<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.sha256)
    }
    #[getter]
    fn metadata(&self) -> Option<PyScCertificateMetadata> {
        self.0.metadata.clone().map(PyScCertificateMetadata)
    }
    #[getter]
    fn validity(&self) -> &'static str {
        self.0.validity.as_str()
    }
    #[getter]
    fn expiry(&self) -> &'static str {
        self.0.expiry.as_str()
    }
    #[getter]
    fn san(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.san.clone())
    }
    #[getter]
    fn key_usage(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.key_usage.clone())
    }
    #[getter]
    fn extended_key_usage(&self) -> PyScExtensionInspection {
        PyScExtensionInspection(self.0.extended_key_usage.clone())
    }
    #[getter]
    fn findings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.0
                .findings
                .iter()
                .map(|v| Py::new(py, PyScInspectionFinding(*v)))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "ScInspectedCertificate(role='{}', index={}, validity='{}')",
            self.role(),
            self.index(),
            self.validity()
        )
    }
}

#[pyclass(name = "ScInspectionFinding", module = "rusty_bacnet", frozen)]
struct PyScInspectionFinding(ScInspectionFinding);
#[pymethods]
impl PyScInspectionFinding {
    #[getter]
    fn area(&self) -> &'static str {
        self.0.area.as_str()
    }
    #[getter]
    fn kind(&self) -> &'static str {
        self.0.kind.as_str()
    }
    fn __repr__(&self) -> String {
        format!(
            "ScInspectionFinding(area='{}', kind='{}')",
            self.area(),
            self.kind()
        )
    }
}

#[pyclass(name = "ScExtensionInspection", module = "rusty_bacnet", frozen)]
struct PyScExtensionInspection(ScExtensionInspection);
#[pymethods]
impl PyScExtensionInspection {
    #[getter]
    fn assessed(&self) -> bool {
        self.0.assessed
    }
    #[getter]
    fn present(&self) -> bool {
        self.0.present
    }
    #[getter]
    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, &self.0.values)
    }
    fn __repr__(&self) -> String {
        format!(
            "ScExtensionInspection(assessed={}, present={}, value_count={})",
            self.assessed(),
            self.present(),
            self.0.values.len()
        )
    }
}

#[pyclass(name = "ScKeyFilePermissions", module = "rusty_bacnet", frozen)]
struct PyScKeyFilePermissions(ScKeyFilePermissions);
#[pymethods]
impl PyScKeyFilePermissions {
    #[getter]
    fn group_readable(&self) -> bool {
        self.0.group_readable
    }
    #[getter]
    fn world_readable(&self) -> bool {
        self.0.world_readable
    }
    fn __repr__(&self) -> String {
        format!(
            "ScKeyFilePermissions(group_readable={}, world_readable={})",
            self.group_readable(),
            self.world_readable()
        )
    }
}

fn time(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<i64>> {
    value
        .map(|value| {
            if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
                return Err(PyTypeError::new_err(
                    "inspected_at_unix must be an integer Unix second",
                ));
            }
            value.extract()
        })
        .transpose()
}
fn bytes_len(value: &Bound<'_, PyAny>) -> PyResult<usize> {
    if let Ok(value) = value.cast::<PyBytes>() {
        Ok(value.as_bytes().len())
    } else if let Ok(value) = value.cast::<PyByteArray>() {
        Ok(value.len())
    } else {
        Err(PyTypeError::new_err(
            "PEM content must be bytes or bytearray",
        ))
    }
}
fn bytes<'a>(value: &'a Bound<'_, PyAny>) -> PyResult<Cow<'a, [u8]>> {
    if let Ok(value) = value.cast::<PyBytes>() {
        Ok(Cow::Borrowed(value.as_bytes()))
    } else if let Ok(value) = value.cast::<PyByteArray>() {
        Ok(Cow::Owned(value.to_vec()))
    } else {
        Err(PyTypeError::new_err(
            "PEM content must be bytes or bytearray",
        ))
    }
}
fn path(value: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    let value = value
        .py()
        .import("os")?
        .getattr("fspath")?
        .call1((value,))?;
    let value = value
        .cast::<PyString>()
        .map_err(|_| PyTypeError::new_err("paths must be str or os.PathLike[str]"))?;
    value.extract()
}
fn error(error: ScInspectionError) -> PyErr {
    match error {
        ScInspectionError::Io { source, .. } => match source.raw_os_error() {
            Some(code) => {
                // PyO3's generic io::Error conversion keeps the message but
                // drops errno. Invoke OSError's native constructor explicitly.
                #[cfg(windows)]
                {
                    PyOSError::new_err((0, source.to_string(), None::<String>, code))
                }
                #[cfg(not(windows))]
                {
                    PyOSError::new_err((code, source.to_string()))
                }
            }
            None => PyOSError::new_err(source.to_string()),
        },
        error => PyValueError::new_err(error.to_string()),
    }
}

/// Inspect supplied PEM bytes without constructing a client or contacting a hub.
#[pyfunction]
#[pyo3(signature = (identity_pem, *, peer_trust_pem=None, private_key_pem=None, inspected_at_unix=None))]
fn inspect_sc_pem(
    identity_pem: &Bound<'_, PyAny>,
    peer_trust_pem: Option<&Bound<'_, PyAny>>,
    private_key_pem: Option<&Bound<'_, PyAny>>,
    inspected_at_unix: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyScPemInspection> {
    let now = time(inspected_at_unix)?;
    let mut total = 0usize;
    for (role, value) in [
        (ScInputRole::Identity, Some(identity_pem)),
        (ScInputRole::PeerTrust, peer_trust_pem),
        (ScInputRole::PrivateKey, private_key_pem),
    ] {
        if let Some(value) = value {
            total = total.saturating_add(bytes_len(value)?);
            if total > MAX_INPUT_BYTES {
                return Err(error(ScInspectionError::Input {
                    role,
                    index: None,
                    kind: ScInputErrorKind::InputTooLarge,
                }));
            }
        }
    }
    let identity = bytes(identity_pem)?;
    let trust = peer_trust_pem.map(bytes).transpose()?;
    let key = private_key_pem.map(bytes).transpose()?;
    inspection::inspect_sc_pem(&identity, trust.as_deref(), key.as_deref(), now)
        .map(PyScPemInspection)
        .map_err(error)
}

/// Read explicitly selected files once, returning an advisory immutable report.
#[pyfunction]
#[pyo3(signature = (identity_path, *, peer_trust_path=None, private_key_path=None, inspected_at_unix=None))]
fn inspect_sc_pem_files(
    identity_path: &Bound<'_, PyAny>,
    peer_trust_path: Option<&Bound<'_, PyAny>>,
    private_key_path: Option<&Bound<'_, PyAny>>,
    inspected_at_unix: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyScPemInspection> {
    let now = time(inspected_at_unix)?;
    let identity = path(identity_path)?;
    let trust = peer_trust_path.map(path).transpose()?;
    let key = private_key_path.map(path).transpose()?;
    identity_path
        .py()
        .detach(|| {
            inspection::inspect_sc_pem_files(&identity, trust.as_deref(), key.as_deref(), now)
        })
        .map(PyScPemInspection)
        .map_err(error)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyScPemInspection>()?;
    m.add_class::<PyScInspectedCertificate>()?;
    m.add_class::<PyScInspectionFinding>()?;
    m.add_class::<PyScExtensionInspection>()?;
    m.add_class::<PyScKeyFilePermissions>()?;
    m.add_function(wrap_pyfunction!(inspect_sc_pem, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_sc_pem_files, m)?)?;
    Ok(())
}
