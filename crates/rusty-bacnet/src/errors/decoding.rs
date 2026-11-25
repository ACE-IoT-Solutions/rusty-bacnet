//! Structured diagnostics for errors reported by a native decoder.

use super::*;
use crate::types::PyPropertyReadContext;
use bacnet_types::error::{DecodeError, DecodingKind};

create_exception!(rusty_bacnet, BacnetDecodingError, BacnetError);

fn kind_name(kind: DecodingKind) -> &'static str {
    match kind {
        DecodingKind::InvalidEncoding => "invalid_encoding",
        DecodingKind::OutOfRange => "out_of_range",
        DecodingKind::Overflow => "overflow",
        DecodingKind::InvalidTag => "invalid_tag",
        DecodingKind::Missing => "missing",
        DecodingKind::Trailing => "trailing",
        DecodingKind::Unsupported => "unsupported",
    }
}

pub(crate) fn decoding_error(
    fault: DecodeError,
    read_context: Option<PyPropertyReadContext>,
) -> PyErr {
    let error = BacnetDecodingError::new_err(fault.to_string());
    let _ = crate::py_async::attach(|py| {
        let value = error.value(py);
        value.setattr("kind", kind_name(fault.kind))?;
        value.setattr("offset", fault.offset)?;
        value.setattr(
            "context",
            (!fault.context.is_empty()).then_some(fault.context),
        )?;
        value.setattr("label", fault.label)?;
        value.setattr("values", (fault.values[0], fault.values[1]))?;
        value.setattr("index", fault.index)?;
        value.setattr("read_context", read_context)
    });
    error
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let class = m.py().get_type::<BacnetDecodingError>();
    for attr in ["context", "index", "read_context"] {
        class.setattr(attr, m.py().None())?;
    }
    m.add("BacnetDecodingError", class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_responses_and_timeouts_keep_their_own_exception_types() {
        use bacnet_types::enums::{AbortReason, RejectReason};
        use std::time::Duration;
        Python::initialize();
        Python::attach(|py| {
            for (error, expected) in [
                (
                    Error::protocol(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
                    py.get_type::<BacnetProtocolError>(),
                ),
                (
                    Error::reject(RejectReason::INVALID_TAG),
                    py.get_type::<BacnetRejectError>(),
                ),
                (
                    Error::abort(AbortReason::OTHER),
                    py.get_type::<BacnetAbortError>(),
                ),
                (
                    Error::Timeout(Duration::from_secs(1)),
                    py.get_type::<BacnetTimeoutError>(),
                ),
            ] {
                assert!(to_py_err(error).value(py).get_type().is(&expected));
            }
        });
    }

    #[test]
    fn native_decoding_faults_retain_their_fields_and_message() {
        Python::initialize();
        Python::attach(|py| {
            for (kind, token) in [
                (DecodingKind::InvalidEncoding, "invalid_encoding"),
                (DecodingKind::OutOfRange, "out_of_range"),
                (DecodingKind::Overflow, "overflow"),
                (DecodingKind::InvalidTag, "invalid_tag"),
                (DecodingKind::Missing, "missing"),
                (DecodingKind::Trailing, "trailing"),
                (DecodingKind::Unsupported, "unsupported"),
            ] {
                let fault = DecodeError::new(kind, 7, "expected {} octets, got {}")
                    .with_values(5, 2)
                    .within("item")
                    .at_item(3);
                let native = Error::from(fault);
                let message = native.to_string();
                let error = to_py_err(native);
                assert!(error.is_instance_of::<BacnetDecodingError>(py));
                assert!(error.is_instance_of::<BacnetError>(py));
                let value = error.value(py);
                assert_eq!(value.to_string(), message);
                assert_eq!(
                    value.getattr("kind").unwrap().extract::<String>().unwrap(),
                    token
                );
                assert_eq!(
                    value.getattr("offset").unwrap().extract::<usize>().unwrap(),
                    7
                );
                assert_eq!(
                    value
                        .getattr("context")
                        .unwrap()
                        .extract::<String>()
                        .unwrap(),
                    "item"
                );
                assert_eq!(
                    value.getattr("label").unwrap().extract::<String>().unwrap(),
                    fault.label
                );
                assert_eq!(
                    value
                        .getattr("values")
                        .unwrap()
                        .extract::<(u64, u64)>()
                        .unwrap(),
                    (5, 2)
                );
                assert_eq!(value.getattr("index").unwrap().extract::<u16>().unwrap(), 3);
                assert!(value.getattr("read_context").unwrap().is_none());
            }
        });
    }
}
