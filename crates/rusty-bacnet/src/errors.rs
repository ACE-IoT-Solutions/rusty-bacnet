//! Python exception types mapping Rust BACnet errors.

use bacnet_types::error::Error;
use pyo3::create_exception;
use pyo3::prelude::*;

// Exception hierarchy: BacnetError (base) with subtypes
create_exception!(rusty_bacnet, BacnetError, pyo3::exceptions::PyException);
create_exception!(rusty_bacnet, BacnetProtocolError, BacnetError);
create_exception!(rusty_bacnet, BacnetTimeoutError, BacnetError);
create_exception!(rusty_bacnet, BacnetRejectError, BacnetError);
create_exception!(rusty_bacnet, BacnetAbortError, BacnetError);
create_exception!(rusty_bacnet, BacnetNotificationLagError, BacnetError);
create_exception!(rusty_bacnet, BacnetNetworkRejectError, BacnetError);
create_exception!(rusty_bacnet, BacnetBvlcError, BacnetError);
create_exception!(rusty_bacnet, BacnetBbmdControlError, BacnetError);
create_exception!(
    rusty_bacnet,
    BacnetForeignDeviceRegistrationError,
    BacnetBvlcError
);

pub fn bbmd_control_to_py_err(error: bacnet_transport::bip::BbmdControlError) -> PyErr {
    let code = match &error {
        bacnet_transport::bip::BbmdControlError::NotStarted => "not_started",
        bacnet_transport::bip::BbmdControlError::Stopped => "stopped",
        bacnet_transport::bip::BbmdControlError::Invalid(_) => "invalid",
        bacnet_transport::bip::BbmdControlError::Io(_) => "io",
    };
    let py_err = BacnetBbmdControlError::new_err(error.to_string());
    Python::attach(|py| {
        let _ = py_err.value(py).setattr("code", code);
    });
    py_err
}

fn network_reject_py_err(network: u16, reason: u8, message: String) -> PyErr {
    let py_err = BacnetNetworkRejectError::new_err(message);
    Python::attach(|py| {
        let val = py_err.value(py);
        let _ = val.setattr("network", network);
        let _ = val.setattr("reason", reason);
    });
    py_err
}

/// Convert a Rust `Error` into a Python exception.
///
/// Protocol, APDU reject/abort, correlated network reject, and BVLC errors
/// carry structured integer attributes so Python callers can inspect them
/// programmatically without parsing the message string.
pub fn to_py_err(err: Error) -> PyErr {
    match err {
        Error::Protocol { class, code } => {
            let py_err =
                BacnetProtocolError::new_err(format!("BACnet error: class={class} code={code}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("error_class", class);
                let _ = val.setattr("error_code", code);
            });
            py_err
        }
        Error::Timeout(_) => BacnetTimeoutError::new_err(err.to_string()),
        Error::Reject { reason } => {
            let py_err = BacnetRejectError::new_err(format!("BACnet reject: reason={reason}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("reason", reason);
            });
            py_err
        }
        Error::Abort { reason } => {
            let py_err = BacnetAbortError::new_err(format!("BACnet abort: reason={reason}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("reason", reason);
            });
            py_err
        }
        Error::NetworkReject { network, reason } => {
            let raw_reason = reason.to_raw();
            network_reject_py_err(
                network,
                raw_reason,
                format!("BACnet route rejected: network={network}, reason={raw_reason}"),
            )
        }
        Error::RoutedPathTooLong { dnet } => network_reject_py_err(
            dnet,
            bacnet_types::enums::RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
            err.to_string(),
        ),
        Error::Bvlc { result_code } => {
            let raw = result_code.to_raw();
            let py_err = if result_code
                == bacnet_types::enums::BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
            {
                BacnetForeignDeviceRegistrationError::new_err(format!(
                    "foreign-device registration rejected: {result_code:?}"
                ))
            } else {
                BacnetBvlcError::new_err(err.to_string())
            };
            Python::attach(|py| {
                let _ = py_err.value(py).setattr("result_code", raw);
            });
            py_err
        }
        _ => BacnetError::new_err(err.to_string()),
    }
}

/// Register exception types with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("BacnetError", m.py().get_type::<BacnetError>())?;
    m.add(
        "BacnetProtocolError",
        m.py().get_type::<BacnetProtocolError>(),
    )?;
    m.add(
        "BacnetTimeoutError",
        m.py().get_type::<BacnetTimeoutError>(),
    )?;
    m.add("BacnetRejectError", m.py().get_type::<BacnetRejectError>())?;
    m.add("BacnetAbortError", m.py().get_type::<BacnetAbortError>())?;
    m.add(
        "BacnetNotificationLagError",
        m.py().get_type::<BacnetNotificationLagError>(),
    )?;
    m.add(
        "BacnetNetworkRejectError",
        m.py().get_type::<BacnetNetworkRejectError>(),
    )?;
    m.add("BacnetBvlcError", m.py().get_type::<BacnetBvlcError>())?;
    m.add(
        "BacnetBbmdControlError",
        m.py().get_type::<BacnetBbmdControlError>(),
    )?;
    m.add(
        "BacnetForeignDeviceRegistrationError",
        m.py().get_type::<BacnetForeignDeviceRegistrationError>(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{BvlcResultCode, RejectMessageReason};

    #[test]
    fn network_reject_keeps_structured_attributes() {
        Python::initialize();
        let py_err = to_py_err(Error::NetworkReject {
            network: 2001,
            reason: RejectMessageReason::from_raw(0xFE),
        });

        Python::attach(|py| {
            let value = py_err.value(py);
            assert!(value.is_instance_of::<BacnetNetworkRejectError>());
            assert_eq!(
                value.getattr("network").unwrap().extract::<u16>().unwrap(),
                2001
            );
            assert_eq!(
                value.getattr("reason").unwrap().extract::<u8>().unwrap(),
                0xFE
            );
        });
    }

    #[test]
    fn upstream_path_too_long_uses_network_reject_taxonomy() {
        Python::initialize();
        let py_err = to_py_err(Error::RoutedPathTooLong { dnet: 4096 });

        Python::attach(|py| {
            let value = py_err.value(py);
            assert!(value.is_instance_of::<BacnetNetworkRejectError>());
            assert_eq!(
                value.getattr("network").unwrap().extract::<u16>().unwrap(),
                4096
            );
            assert_eq!(
                value.getattr("reason").unwrap().extract::<u8>().unwrap(),
                RejectMessageReason::MESSAGE_TOO_LONG.to_raw()
            );
        });
    }

    #[test]
    fn foreign_device_nak_has_specific_bvlc_subclass_and_code() {
        Python::initialize();
        let py_err = to_py_err(Error::Bvlc {
            result_code: BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
        });

        Python::attach(|py| {
            let value = py_err.value(py);
            assert!(value.is_instance_of::<BacnetForeignDeviceRegistrationError>());
            assert!(value.is_instance_of::<BacnetBvlcError>());
            assert_eq!(
                value
                    .getattr("result_code")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK.to_raw()
            );
        });
    }

    #[test]
    fn other_bvlc_results_keep_the_generic_bvlc_subclass() {
        Python::initialize();
        let py_err = to_py_err(Error::Bvlc {
            result_code: BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
        });

        Python::attach(|py| {
            let value = py_err.value(py);
            assert!(value.is_instance_of::<BacnetBvlcError>());
            assert!(!value.is_instance_of::<BacnetForeignDeviceRegistrationError>());
            assert_eq!(
                value
                    .getattr("result_code")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK.to_raw()
            );
        });
    }

    #[test]
    fn unrelated_upstream_errors_keep_the_generic_fallback() {
        Python::initialize();
        let py_err = to_py_err(Error::RoutedPathCapacityExceeded { capacity: 32 });

        Python::attach(|py| {
            let value = py_err.value(py);
            assert!(value.is_instance_of::<BacnetError>());
            assert!(!value.is_instance_of::<BacnetNetworkRejectError>());
            assert!(value.str().unwrap().to_str().unwrap().contains("32"));
        });
    }
}
