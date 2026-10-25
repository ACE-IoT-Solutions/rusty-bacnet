//! The Python form of a BACnet bit string (#2080), and the name helpers that
//! reuse `bacnet_types::enums::resolve_enum` / `resolve_bits`.
//!
//! A `BitString` keeps the wire form (`unused_bits`, `data`) and adds `bits`,
//! one bool per bit in wire order (bit 0 is the top bit of the first octet),
//! and `names`, the set bits by their constant names (`"IN_ALARM"`,
//! `"TO_OFFNORMAL"`) when the bit string's type is known from where it was
//! read, or `None` when it isn't. Names are the same UPPER_SNAKE spellings the
//! enum classes use for their attributes.

use bacnet_types::bitstring::{
    EventTransitionBits, LimitEnable, LogStatus, ObjectTypesSupported, ServicesSupported,
    StatusFlags,
};
use bacnet_types::enums::{PropertyIdentifier, ResolvedBits};

use pyo3::types::PyTuple;

use super::*;

/// A BACnet bit string: the wire form plus its bits and, when its type is
/// known, the names of the set bits.
#[pyclass(name = "BitString", module = "rusty_bacnet", frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PyBitString {
    unused_bits: u8,
    data: Vec<u8>,
    names: Option<Vec<String>>,
}

impl PyBitString {
    /// A bit string of no known type: `names` is `None`.
    pub(crate) fn raw(unused_bits: u8, data: &[u8]) -> Self {
        Self {
            unused_bits,
            data: data.to_vec(),
            names: None,
        }
    }

    /// The bit string read from `property`: named when `resolve_bits` knows
    /// the property's bit-string type.
    pub(crate) fn for_property(
        property: Option<PropertyIdentifier>,
        unused_bits: u8,
        data: &[u8],
    ) -> Self {
        let names = property.and_then(|property| {
            resolved_bit_names(&ResolvedBits::from_property(property, unused_bits, data))
        });
        Self {
            unused_bits,
            data: data.to_vec(),
            names,
        }
    }

    /// A `BACnetEventTransitionBits` value (`acked-transitions` and kin), in
    /// its three-bit wire form.
    pub(crate) fn event_transitions(bits: EventTransitionBits) -> Self {
        Self {
            unused_bits: 5,
            data: vec![bits.to_bacnet()],
            names: Some(flag_names(bits.iter_names())),
        }
    }

    /// A `BACnetStatusFlags` value, in its four-bit wire form.
    pub(crate) fn status_flags(flags: StatusFlags) -> Self {
        // StatusFlags keeps IN_ALARM in its 0b1000 bit; the wire wants it in
        // the top bit of the octet, so shift the nibble up.
        Self {
            unused_bits: 4,
            data: vec![flags.bits() << 4],
            names: Some(flag_names(flags.iter_names())),
        }
    }

    /// A `BACnetLogStatus` value, in its three-bit wire form.
    pub(crate) fn log_status(status: LogStatus) -> Self {
        Self {
            unused_bits: 5,
            data: vec![status.to_bacnet()],
            names: Some(flag_names(status.iter_names())),
        }
    }

    fn bit_count(&self) -> usize {
        (self.data.len() * 8).saturating_sub(usize::from(self.unused_bits))
    }

    fn bit(&self, n: usize) -> bool {
        self.data
            .get(n / 8)
            .is_some_and(|octet| octet & (0x80 >> (n % 8)) != 0)
    }
}

fn flag_names<T>(names: impl Iterator<Item = (&'static str, T)>) -> Vec<String> {
    names.map(|(name, _)| name.to_owned()).collect()
}

/// The set bits' names for a resolved bit string, or `None` for one of no
/// known type. A set bit with no name (a vendor or reserved position) is left
/// out; `bits` still shows it.
fn resolved_bit_names(bits: &ResolvedBits) -> Option<Vec<String>> {
    Some(match bits {
        ResolvedBits::StatusFlags(flags) => flag_names(flags.iter_names()),
        ResolvedBits::EventTransitionBits(bits) => flag_names(bits.iter_names()),
        ResolvedBits::LimitEnable(bits) => flag_names(LimitEnable::iter_names(bits)),
        ResolvedBits::ServicesSupported(services) => ServicesSupported::iter(services)
            .filter_map(|s| s.name().map(str::to_owned))
            .collect(),
        ResolvedBits::ObjectTypesSupported(types) => ObjectTypesSupported::iter(types)
            .filter_map(|t| t.name().map(str::to_owned))
            .collect(),
        ResolvedBits::Unknown { .. } => return None,
    })
}

#[pymethods]
impl PyBitString {
    /// A bit string from its wire form. With `property`, the bits are named
    /// as that property's bit-string type names them (`STATUS_FLAGS`,
    /// `EVENT_ENABLE`, `PROTOCOL_SERVICES_SUPPORTED` and so on); without one,
    /// or for a property of no known bit-string type, `names` is `None`.
    /// `unused_bits` is 0 to 7, and 0 when `data` is empty.
    #[new]
    #[pyo3(signature = (unused_bits, data, property=None))]
    fn new(
        unused_bits: u8,
        data: Vec<u8>,
        property: Option<PyPropertyIdentifier>,
    ) -> PyResult<Self> {
        if unused_bits > 7 || (data.is_empty() && unused_bits != 0) {
            return Err(PyValueError::new_err(
                "unused_bits must be 0 to 7, and 0 for an empty bit string",
            ));
        }
        Ok(Self::for_property(
            property.map(|p| p.to_rust()),
            unused_bits,
            &data,
        ))
    }

    /// How many trailing bits of the final octet of `data` are padding.
    #[getter]
    fn unused_bits(&self) -> u8 {
        self.unused_bits
    }

    /// The bit string's octets as carried on the wire.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// One bool per bit, bit 0 first.
    #[getter]
    fn bits(&self) -> Vec<bool> {
        (0..self.bit_count()).map(|n| self.bit(n)).collect()
    }

    /// The set bits by name when the type is known, else `None`.
    #[getter]
    fn names(&self) -> Option<Vec<String>> {
        self.names.clone()
    }

    fn __len__(&self) -> usize {
        self.bit_count()
    }

    fn __repr__(&self) -> String {
        let bits: String = (0..self.bit_count())
            .map(|n| if self.bit(n) { '1' } else { '0' })
            .collect();
        match &self.names {
            Some(names) => {
                // Names are UPPER_SNAKE identifiers, so Python's quoting is plain.
                let names: Vec<String> = names.iter().map(|name| format!("'{name}'")).collect();
                format!("BitString('{bits}', names=[{}])", names.join(", "))
            }
            None => format!("BitString('{bits}')"),
        }
    }

    /// Equal when the wire forms are equal; `names` follows from where a
    /// value was read and doesn't take part.
    fn __eq__(&self, other: &Self) -> bool {
        self.unused_bits == other.unused_bits && self.data == other.data
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut state = std::collections::hash_map::DefaultHasher::new();
        (self.unused_bits, &self.data).hash(&mut state);
        state.finish()
    }

    /// What `copy` and `pickle` call: `_restore` with the names kept.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let this = slf.get();
        let arguments = (
            this.unused_bits,
            PyBytes::new(py, &this.data),
            this.names.clone(),
        )
            .into_pyobject(py)?;
        Ok((slf.get_type().getattr("_restore")?, arguments))
    }

    /// Rebuild a pickled bit string, names included.
    #[staticmethod]
    fn _restore(unused_bits: u8, data: Vec<u8>, names: Option<Vec<String>>) -> PyResult<Self> {
        let mut restored = Self::new(unused_bits, data, None)?;
        restored.names = names;
        Ok(restored)
    }
}

/// The name of enumerated `value` as read from `property`, or `None` when the
/// property's enumeration isn't known from the property alone or the number
/// has no name in it. Reuses `bacnet_types::enums::resolve_enum`.
#[pyfunction]
pub(crate) fn enum_name(property: PyPropertyIdentifier, value: u32) -> Option<&'static str> {
    resolved_enum_name(property.to_rust(), value)
}

/// The name of enumerated `value` read from `property`, through `resolve_enum`.
pub(crate) fn resolved_enum_name(property: PropertyIdentifier, value: u32) -> Option<&'static str> {
    bacnet_types::enums::resolve_enum(property, &primitives::PropertyValue::Enumerated(value))
        .and_then(|resolved| resolved.name())
}
