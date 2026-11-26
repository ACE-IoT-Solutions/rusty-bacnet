//! Where a successful ReadProperty or ReadPropertyMultiple value came from.
//!
//! The context names the object, property and array index the ACK names,
//! which can differ from the request (a wildcard Device instance, for one).
//! ACK contexts keep the exact value bytes before projection.

use super::{PyObjectIdentifier, PyPropertyValue};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};
use std::hash::{Hash, Hasher};

/// The object, property and optional array index the ACK names. A value
/// class: equal when all three are, hashable, and picklable on its own.
#[pyclass(
    name = "PropertyReadContext",
    module = "rusty_bacnet",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
pub(crate) struct PyPropertyReadContext {
    object: ObjectIdentifier,
    #[pyo3(get)]
    pub(super) property_id: u32,
    #[pyo3(get)]
    pub(super) array_index: Option<u32>,
    octets: Option<Bytes>,
}

impl From<bacnet_services::read_property::ReadPropertyACK> for PyPropertyReadContext {
    fn from(ack: bacnet_services::read_property::ReadPropertyACK) -> Self {
        Self::new(
            ack.object_identifier,
            ack.property_identifier.to_raw(),
            ack.property_array_index,
            ack.property_value.into(),
        )
    }
}

impl PyPropertyReadContext {
    pub(super) fn new(
        object: ObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        octets: Bytes,
    ) -> Self {
        Self {
            object,
            property_id,
            array_index,
            octets: Some(octets),
        }
    }
}
impl PyPropertyReadContext {
    fn selector(&self) -> (ObjectIdentifier, u32, Option<u32>) {
        (self.object, self.property_id, self.array_index)
    }
}
impl PartialEq for PyPropertyReadContext {
    fn eq(&self, other: &Self) -> bool {
        self.selector() == other.selector()
    }
}
impl Eq for PyPropertyReadContext {}
impl Hash for PyPropertyReadContext {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.selector().hash(state);
    }
}

#[pymethods]
impl PyPropertyReadContext {
    /// The object identifier the ACK names.
    #[getter]
    pub(super) fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// Exact value bytes when this context came from an ACK.
    #[getter]
    pub(super) fn raw<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.octets.as_ref().map(|bytes| PyBytes::new(py, bytes))
    }

    fn __repr__(&self) -> String {
        let index = self
            .array_index
            .map_or_else(|| "None".to_owned(), |index| index.to_string());
        format!(
            "PropertyReadContext(object_id=ObjectIdentifier({}, {}), property_id={}, array_index={index})",
            self.object.object_type(),
            self.object.instance_number(),
            self.property_id,
        )
    }

    /// What `copy` and `pickle` call: `_restore` with the three fields.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let this = slf.get();
        let arguments = (
            this.object_id(),
            this.property_id,
            this.array_index,
            this.raw(slf.py()),
        )
            .into_pyobject(slf.py())?;
        Ok((slf.get_type().getattr("_restore")?, arguments))
    }

    /// Rebuild a pickled context.
    #[staticmethod]
    #[pyo3(signature = (object_id, property_id, array_index, raw=None))]
    fn _restore(
        object_id: PyObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        raw: Option<&[u8]>,
    ) -> Self {
        Self {
            object: object_id.to_rust(),
            property_id,
            array_index,
            octets: raw.map(Bytes::copy_from_slice),
        }
    }
}

impl PyPropertyValue {
    /// This value as read by an RP or RPM whose ACK names `object`,
    /// `property_id` and `array_index`.
    pub(super) fn with_read_context(
        mut self,
        object: ObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        raw: &[u8],
    ) -> Self {
        self.context = Some(PyPropertyReadContext {
            object,
            property_id,
            array_index,
            octets: Some(Bytes::copy_from_slice(raw)),
        });
        self
    }
}

#[pymethods]
impl PyPropertyValue {
    /// The object, property and array index the ACK of a network
    /// ReadProperty or ReadPropertyMultiple named; None for any other value.
    #[getter]
    fn read_context(&self) -> Option<PyPropertyReadContext> {
        self.context.clone()
    }

    /// What `__reduce__` rebuilds a read value with, so a copy keeps its
    /// context.
    #[staticmethod]
    #[pyo3(signature = (value, object_id, property_id, array_index, raw=None))]
    fn _with_read_context(
        mut value: Self,
        object_id: PyObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        raw: Option<&[u8]>,
    ) -> Self {
        value.context = Some(PyPropertyReadContext::_restore(
            object_id,
            property_id,
            array_index,
            raw,
        ));
        value
    }
}
