use bacnet_encoding::{primitives as encoding, tags};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use super::PyPropertyValue;

const MAX_TAG_DEPTH: usize = tags::MAX_CONTEXT_NESTING_DEPTH;

/// A decoded BACnet tag and its exact wire bytes.
#[pyclass(name = "RawTag", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRawTag {
    tag_class: &'static str,
    tag_number: u8,
    length: u32,
    content: Vec<u8>,
    header: Vec<u8>,
    full_tlv: Vec<u8>,
    opening: bool,
    closing: bool,
    depth: usize,
}

#[pymethods]
impl PyRawTag {
    #[getter]
    fn tag_class(&self) -> &'static str {
        self.tag_class
    }

    #[getter]
    fn tag_number(&self) -> u8 {
        self.tag_number
    }

    #[getter]
    fn length(&self) -> u32 {
        self.length
    }

    #[getter]
    fn content<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.content)
    }

    #[getter]
    fn header<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.header)
    }

    #[getter]
    fn full_tlv<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.full_tlv)
    }

    #[getter]
    fn opening(&self) -> bool {
        self.opening
    }

    #[getter]
    fn closing(&self) -> bool {
        self.closing
    }

    #[getter]
    fn depth(&self) -> usize {
        self.depth
    }

    fn __repr__(&self) -> String {
        format!(
            "RawTag(tag_class={:?}, tag_number={}, length={}, opening={}, closing={}, depth={})",
            self.tag_class, self.tag_number, self.length, self.opening, self.closing, self.depth
        )
    }
}

fn value_error(context: &str, error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("{context}: {error}"))
}

fn canonical_hint(hint: &str) -> PyResult<&str> {
    if let Some(rest) = hint.strip_prefix("prop:") {
        let (property, datatype) = rest.split_once(':').ok_or_else(|| {
            PyValueError::new_err("property hint must have the form 'prop:<u32>:<hint>'")
        })?;
        if property.is_empty() || datatype.is_empty() || datatype.contains(':') {
            return Err(PyValueError::new_err(
                "property hint must have the form 'prop:<u32>:<hint>'",
            ));
        }
        property.parse::<u32>().map_err(|_| {
            PyValueError::new_err("property hint identifier must be an unsigned 32-bit integer")
        })?;
        Ok(datatype)
    } else {
        Ok(hint)
    }
}

fn require_len(content: &[u8], expected: usize, hint: &str) -> PyResult<()> {
    if content.len() == expected {
        Ok(())
    } else {
        Err(PyValueError::new_err(format!(
            "{hint} requires exactly {expected} content bytes, got {}",
            content.len()
        )))
    }
}

fn validate_wire_tag_header(raw: &[u8], offset: usize) -> PyResult<()> {
    let first = *raw
        .get(offset)
        .ok_or_else(|| PyValueError::new_err("missing BACnet tag header"))?;
    let context = first & 0x08 != 0;
    let lvt = first & 0x07;
    if !context && matches!(lvt, 6 | 7) {
        return Err(PyValueError::new_err(
            "application tags cannot use opening or closing LVT values",
        ));
    }
    if first >> 4 == 0x0f {
        let extended = *raw
            .get(offset + 1)
            .ok_or_else(|| PyValueError::new_err("missing extended tag number"))?;
        if extended < 15 {
            return Err(PyValueError::new_err(format!(
                "non-canonical extended tag number {extended}; values below 15 must use the short form"
            )));
        }
    }
    Ok(())
}

fn validate_date(content: &[u8]) -> PyResult<()> {
    let month = content[1];
    let day = content[2];
    let weekday = content[3];
    if !matches!(month, 1..=14 | 0xff) {
        return Err(PyValueError::new_err(format!(
            "date month must be 1-14 or 255, got {month}"
        )));
    }
    if !matches!(day, 1..=34 | 0xff) {
        return Err(PyValueError::new_err(format!(
            "date day must be 1-34 or 255, got {day}"
        )));
    }
    if !matches!(weekday, 1..=7 | 0xff) {
        return Err(PyValueError::new_err(format!(
            "date weekday must be 1-7 or 255, got {weekday}"
        )));
    }
    Ok(())
}

fn validate_time(content: &[u8]) -> PyResult<()> {
    let limits = [
        ("hour", 23),
        ("minute", 59),
        ("second", 59),
        ("hundredths", 99),
    ];
    for ((name, maximum), value) in limits.into_iter().zip(content.iter().copied()) {
        if value != 0xff && value > maximum {
            return Err(PyValueError::new_err(format!(
                "time {name} must be at most {maximum} or 255, got {value}"
            )));
        }
    }
    Ok(())
}

fn decode_content(content: &[u8], hint: &str, boolean_lvt: Option<u32>) -> PyResult<PropertyValue> {
    let value = match hint {
        "null" => {
            require_len(content, 0, hint)?;
            PropertyValue::Null
        }
        "boolean" => {
            let value = if let Some(lvt) = boolean_lvt {
                match lvt {
                    0 => false,
                    1 => true,
                    _ => {
                        return Err(PyValueError::new_err(format!(
                            "application Boolean LVT must be 0 or 1, got {lvt}"
                        )));
                    }
                }
            } else {
                require_len(content, 1, hint)?;
                match content[0] {
                    0 => false,
                    1 => true,
                    value => {
                        return Err(PyValueError::new_err(format!(
                            "context/proprietary Boolean content must be 0 or 1, got {value}"
                        )));
                    }
                }
            };
            PropertyValue::Boolean(value)
        }
        "unsigned" => PropertyValue::Unsigned(
            encoding::decode_unsigned(content).map_err(|e| value_error("invalid unsigned", e))?,
        ),
        "signed" => PropertyValue::Signed(
            encoding::decode_signed(content).map_err(|e| value_error("invalid signed", e))?,
        ),
        "real" => PropertyValue::Real(
            encoding::decode_real(content).map_err(|e| value_error("invalid real", e))?,
        ),
        "double" => PropertyValue::Double(
            encoding::decode_double(content).map_err(|e| value_error("invalid double", e))?,
        ),
        "octet_string" => PropertyValue::OctetString(content.to_vec()),
        "character_string" => PropertyValue::CharacterString(
            encoding::decode_character_string(content)
                .map_err(|e| value_error("invalid character_string", e))?,
        ),
        "bit_string" => {
            let (unused_bits, data) = encoding::decode_bit_string(content)
                .map_err(|e| value_error("invalid bit_string", e))?;
            if data.is_empty() && unused_bits != 0 {
                return Err(PyValueError::new_err(
                    "empty bit_string must have zero unused bits",
                ));
            }
            if unused_bits > 0
                && data
                    .last()
                    .is_some_and(|last| last & ((1u8 << unused_bits) - 1) != 0)
            {
                return Err(PyValueError::new_err(
                    "bit_string unused low bits in the final octet must be zero",
                ));
            }
            PropertyValue::BitString { unused_bits, data }
        }
        "enumerated" => {
            if content.is_empty() || content.len() > 4 {
                return Err(PyValueError::new_err(format!(
                    "enumerated requires 1-4 content bytes, got {}",
                    content.len()
                )));
            }
            PropertyValue::Enumerated(
                encoding::decode_unsigned(content)
                    .map_err(|e| value_error("invalid enumerated", e))? as u32,
            )
        }
        "date" => {
            require_len(content, 4, hint)?;
            validate_date(content)?;
            PropertyValue::Date(Date::decode(content).map_err(|e| value_error("invalid date", e))?)
        }
        "time" => {
            require_len(content, 4, hint)?;
            validate_time(content)?;
            PropertyValue::Time(Time::decode(content).map_err(|e| value_error("invalid time", e))?)
        }
        "object_identifier" => {
            require_len(content, 4, hint)?;
            PropertyValue::ObjectIdentifier(
                ObjectIdentifier::decode(content)
                    .map_err(|e| value_error("invalid object_identifier", e))?,
            )
        }
        _ => {
            return Err(PyValueError::new_err(format!(
                "unknown datatype hint {hint:?}; expected null, boolean, unsigned, signed, real, double, octet_string, character_string, bit_string, enumerated, date, time, or object_identifier"
            )));
        }
    };
    Ok(value)
}

/// Decode one complete BACnet TLV using a caller-supplied datatype.
#[pyfunction]
pub fn decode_raw_value(raw: &[u8], hint: &str) -> PyResult<PyPropertyValue> {
    if raw.is_empty() {
        return Err(PyValueError::new_err("raw value is empty"));
    }
    validate_wire_tag_header(raw, 0)?;
    let hint = canonical_hint(hint)?;
    let (tag, content_start) =
        tags::decode_tag(raw, 0).map_err(|e| value_error("invalid BACnet tag header", e))?;
    if tag.is_opening || tag.is_closing {
        return Err(PyValueError::new_err(
            "decode_raw_value requires one primitive TLV, not an opening or closing tag",
        ));
    }

    let application_boolean =
        tag.class == tags::TagClass::Application && tag.number == tags::app_tag::BOOLEAN;
    let raw_lvt = raw[0] & 0x07;
    if application_boolean && raw_lvt > 1 {
        return Err(PyValueError::new_err(format!(
            "application Boolean LVT must be 0 or 1, got {raw_lvt}"
        )));
    }
    let content_end = if application_boolean {
        content_start
    } else {
        content_start
            .checked_add(tag.length as usize)
            .ok_or_else(|| PyValueError::new_err("tag content length overflow"))?
    };
    if content_end > raw.len() {
        return Err(PyValueError::new_err(format!(
            "tag declares {} content bytes but only {} remain",
            tag.length,
            raw.len().saturating_sub(content_start)
        )));
    }
    if content_end != raw.len() {
        return Err(PyValueError::new_err(format!(
            "raw value contains {} trailing bytes after the TLV",
            raw.len() - content_end
        )));
    }
    if tag.class == tags::TagClass::Application && tag.number == tags::app_tag::NULL && raw_lvt != 0
    {
        return Err(PyValueError::new_err("application Null must have LVT 0"));
    }

    let value = decode_content(
        &raw[content_start..content_end],
        hint,
        application_boolean.then_some(tag.length),
    )?;
    Ok(PyPropertyValue::from_rust(value))
}

/// Describe every tag in a BACnet tag stream without interpreting datatypes.
#[pyfunction]
pub fn describe_tags(raw: &[u8]) -> PyResult<Vec<PyRawTag>> {
    let mut entries = Vec::new();
    let mut stack: Vec<u8> = Vec::new();
    let mut offset = 0usize;

    while offset < raw.len() {
        validate_wire_tag_header(raw, offset)?;
        let (tag, content_start) = tags::decode_tag(raw, offset)
            .map_err(|e| value_error("invalid BACnet tag stream", e))?;
        let header = raw[offset..content_start].to_vec();
        let tag_class = match tag.class {
            tags::TagClass::Application => "application",
            tags::TagClass::Context => "context",
        };

        if tag.is_opening {
            if stack.len() >= MAX_TAG_DEPTH {
                return Err(PyValueError::new_err(format!(
                    "context tag nesting depth exceeds maximum ({MAX_TAG_DEPTH})"
                )));
            }
            let depth = stack.len();
            stack.push(tag.number);
            entries.push(PyRawTag {
                tag_class,
                tag_number: tag.number,
                length: 0,
                content: Vec::new(),
                header: header.clone(),
                full_tlv: header,
                opening: true,
                closing: false,
                depth,
            });
            offset = content_start;
            continue;
        }

        if tag.is_closing {
            let expected = stack.pop().ok_or_else(|| {
                PyValueError::new_err(format!("unmatched closing tag {}", tag.number))
            })?;
            if expected != tag.number {
                return Err(PyValueError::new_err(format!(
                    "closing tag {} does not match opening tag {expected}",
                    tag.number
                )));
            }
            let depth = stack.len();
            entries.push(PyRawTag {
                tag_class,
                tag_number: tag.number,
                length: 0,
                content: Vec::new(),
                header: header.clone(),
                full_tlv: header,
                opening: false,
                closing: true,
                depth,
            });
            offset = content_start;
            continue;
        }

        let application_boolean =
            tag.class == tags::TagClass::Application && tag.number == tags::app_tag::BOOLEAN;
        let raw_lvt = raw[offset] & 0x07;
        if application_boolean && raw_lvt > 1 {
            return Err(PyValueError::new_err(format!(
                "application Boolean LVT must be 0 or 1, got {raw_lvt}"
            )));
        }
        if tag.class == tags::TagClass::Application
            && tag.number == tags::app_tag::NULL
            && raw_lvt != 0
        {
            return Err(PyValueError::new_err("application Null must have LVT 0"));
        }
        let content_end = if application_boolean {
            content_start
        } else {
            content_start
                .checked_add(tag.length as usize)
                .ok_or_else(|| PyValueError::new_err("tag content length overflow"))?
        };
        if content_end > raw.len() {
            return Err(PyValueError::new_err(format!(
                "tag {} declares {} content bytes but only {} remain",
                tag.number,
                tag.length,
                raw.len().saturating_sub(content_start)
            )));
        }
        entries.push(PyRawTag {
            tag_class,
            tag_number: tag.number,
            length: tag.length,
            content: raw[content_start..content_end].to_vec(),
            header,
            full_tlv: raw[offset..content_end].to_vec(),
            opening: false,
            closing: false,
            depth: stack.len(),
        });
        offset = content_end;
    }

    if let Some(tag_number) = stack.last() {
        return Err(PyValueError::new_err(format!(
            "missing closing tag {tag_number}"
        )));
    }
    Ok(entries)
}
