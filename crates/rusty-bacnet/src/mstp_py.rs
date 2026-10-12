//! Helpers for constructing MS/TP transports from Python kwargs.

use std::time::Duration;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::{Echo, MstpConfig, MstpTransport};
use bacnet_transport::mstp_serial::{self, SerialConfig, TokioSerialPort};
use bacnet_types::error::Error;

/// Serial port type used by the Python bindings' [`AnyTransport`] parameter.
pub type PySerial = TokioSerialPort;

const SUPPORTED_MSTP_BAUD_RATES: [u32; 6] = [9_600, 19_200, 38_400, 57_600, 76_800, 115_200];
const SUPPORTED_MSTP_BAUD_ERROR: &str =
    "mstp_baud must be one of 9600, 19200, 38400, 57600, 76800, or 115200";

fn is_supported_mstp_baud(rate: u32) -> bool {
    SUPPORTED_MSTP_BAUD_RATES.contains(&rate)
}

/// Validate Python-owned MS/TP configuration without opening the serial device.
pub(crate) fn validate_mstp_config(
    serial_port: Option<&str>,
    mstp_baud: u32,
    mstp_mac: u8,
    mstp_max_master: u8,
    mstp_max_info_frames: u8,
) -> PyResult<&str> {
    let path = serial_port
        .ok_or_else(|| PyValueError::new_err("serial_port is required for transport='mstp'"))?;
    if !is_supported_mstp_baud(mstp_baud) {
        return Err(PyValueError::new_err(SUPPORTED_MSTP_BAUD_ERROR));
    }
    if mstp_mac > 127 {
        return Err(PyValueError::new_err("mstp_mac must be in 0..=127"));
    }
    if mstp_max_master > 127 {
        return Err(PyValueError::new_err("mstp_max_master must be in 0..=127"));
    }
    if mstp_mac > mstp_max_master {
        return Err(PyValueError::new_err("mstp_mac must be <= mstp_max_master"));
    }
    if mstp_max_info_frames == 0 {
        return Err(PyValueError::new_err(
            "mstp_max_info_frames must be in 1..=255",
        ));
    }
    Ok(path)
}

const ECHO_GRACE_WITHOUT_ECHO_ERROR: &str = "mstp_echo_grace_ms needs mstp_echo=True";
const ZERO_ECHO_GRACE_ERROR: &str = "mstp_echo_grace_ms must be at least 1";

/// The MS/TP keywords of `BACnetClient` and `BACnetServer`, kept as given:
/// they are checked when the transport is built, before the port opens.
#[derive(Debug, Clone)]
pub(crate) struct MstpOptions {
    pub(crate) serial_port: Option<String>,
    pub(crate) baud: u32,
    pub(crate) mac: u8,
    pub(crate) max_master: u8,
    pub(crate) max_info_frames: u8,
    /// Extended frames (Types 32 and 33, NPDUs up to 1497 octets), off by
    /// default (#1611).
    pub(crate) extended_frames: bool,
    /// The adapter returns every octet this station sends, and the
    /// transport strips that echo (#1653).
    pub(crate) echo: bool,
    /// How long after a frame its echo may still be arriving, in
    /// milliseconds; `None` is the transport's default grace.
    pub(crate) echo_grace_ms: Option<u32>,
}

impl MstpOptions {
    /// What the port's receiver hears while it transmits, as the echo
    /// keywords declare it. A grace needs the echo on, and must not be zero,
    /// which [`Echo::validate`] refuses too.
    fn port_echo(&self) -> PyResult<Echo> {
        let echo = match (self.echo, self.echo_grace_ms) {
            (false, None) => Echo::None,
            (false, Some(_)) => {
                return Err(PyValueError::new_err(ECHO_GRACE_WITHOUT_ECHO_ERROR));
            }
            (true, None) => Echo::full(),
            (true, Some(grace)) => Echo::Full {
                grace: Duration::from_millis(u64::from(grace)),
            },
        };
        echo.validate()
            .map_err(|_| PyValueError::new_err(ZERO_ECHO_GRACE_ERROR))?;
        Ok(echo)
    }

    fn config(&self) -> MstpConfig {
        MstpConfig {
            this_station: self.mac,
            max_master: self.max_master,
            max_info_frames: self.max_info_frames,
            baud_rate: self.baud,
            extended_frames: self.extended_frames,
        }
    }
}

/// Open an MS/TP transport from Python kwargs, all checked before the port
/// opens.
pub(crate) fn build_mstp_transport(options: &MstpOptions) -> PyResult<AnyTransport<PySerial>> {
    let path = validate_mstp_config(
        options.serial_port.as_deref(),
        options.baud,
        options.mac,
        options.max_master,
        options.max_info_frames,
    )?;
    let echo = options.port_echo()?;
    let serial = open_serial(path, options.baud, echo)?;
    Ok(AnyTransport::Mstp(MstpTransport::new(
        serial,
        options.config(),
    )))
}

/// Open the serial port at `path`, declaring `echo`.
fn open_serial(path: &str, baud: u32, echo: Echo) -> PyResult<TokioSerialPort> {
    let serial = TokioSerialPort::open(&SerialConfig {
        port_name: path.to_string(),
        baud_rate: baud,
    })
    .map_err(serial_error)?;
    Ok(serial.with_echo(echo))
}

/// A serial failure as RuntimeError, with the serial layer's own message.
fn serial_error(error: Error) -> PyErr {
    let message = match error {
        Error::Encoding(message) => message,
        other => other.to_string(),
    };
    PyRuntimeError::new_err(message)
}

/// List the serial ports the operating system reports, as names to pass as
/// `serial_port=` for `transport="mstp"`.
///
/// macOS lists them through IOKit, Windows through SetupAPI and the registry,
/// and Linux from sysfs. Raises OSError (or the subclass for the failure's
/// kind) if the operating system can't be asked.
#[pyfunction]
pub fn list_serial_ports(py: Python<'_>) -> PyResult<Vec<String>> {
    py.detach(mstp_serial::available_ports)
        .map_err(|error| match error {
            Error::Transport(io) => PyErr::from(io),
            other => serial_error(other),
        })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pyo3::exceptions::{PyRuntimeError, PyValueError};
    use pyo3::{PyErr, Python};

    use super::*;

    fn assert_py_error<T>(result: PyResult<T>, message: &str, is_value_error: bool) {
        let err = match result {
            Ok(_) => panic!("expected Python error"),
            Err(err) => err,
        };
        Python::attach(|py| {
            if is_value_error {
                assert!(err.is_instance_of::<PyValueError>(py));
            } else {
                assert!(err.is_instance_of::<PyRuntimeError>(py));
            }
            assert_eq!(err.value(py).to_string(), message);
        });
    }

    fn validate(
        baud: u32,
        mac: u8,
        max_master: u8,
        max_info_frames: u8,
    ) -> Result<&'static str, PyErr> {
        validate_mstp_config(
            Some("intentionally-nonexistent-serial-device"),
            baud,
            mac,
            max_master,
            max_info_frames,
        )
    }

    #[test]
    fn rejects_invalid_configuration_before_open() {
        Python::initialize();
        assert_py_error(
            validate_mstp_config(None, 38_400, 1, 127, 1),
            "serial_port is required for transport='mstp'",
            true,
        );
        for baud in [0, 12_345] {
            assert_py_error(validate(baud, 1, 127, 1), SUPPORTED_MSTP_BAUD_ERROR, true);
        }
        assert_py_error(
            validate(38_400, 128, 127, 1),
            "mstp_mac must be in 0..=127",
            true,
        );
        assert_py_error(
            validate(38_400, 1, 128, 1),
            "mstp_max_master must be in 0..=127",
            true,
        );
        assert_py_error(
            validate(38_400, 4, 3, 1),
            "mstp_mac must be <= mstp_max_master",
            true,
        );
        assert_py_error(
            validate(38_400, 1, 127, 0),
            "mstp_max_info_frames must be in 1..=255",
            true,
        );
    }

    #[test]
    fn accepts_all_supported_baud_rates() {
        for baud in SUPPORTED_MSTP_BAUD_RATES {
            assert!(validate(baud, 1, 127, 1).is_ok(), "baud {baud}");
        }
    }

    fn missing_serial_path() -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir()
            .join(format!("rusty-bacnet-missing-{unique}"))
            .join("serial-device")
            .to_string_lossy()
            .into_owned()
    }

    fn options() -> MstpOptions {
        MstpOptions {
            serial_port: Some(missing_serial_path()),
            baud: 38_400,
            mac: 1,
            max_master: 127,
            max_info_frames: 1,
            extended_frames: false,
            echo: false,
            echo_grace_ms: None,
        }
    }

    #[test]
    fn valid_configuration_reaches_serial_open() {
        Python::initialize();
        let result = build_mstp_transport(&MstpOptions {
            baud: 115_200,
            max_info_frames: 255,
            extended_frames: true,
            echo: true,
            echo_grace_ms: Some(25),
            ..options()
        });
        let err = match result {
            Ok(_) => panic!("nonexistent serial device unexpectedly opened"),
            Err(err) => err,
        };
        Python::attach(|py| {
            assert!(err.is_instance_of::<PyRuntimeError>(py));
            let message = err.value(py).to_string();
            assert!(message.starts_with("Serial open failed"), "{message}");
            assert_eq!(
                message.matches("Serial open failed").count(),
                1,
                "{message}"
            );
        });
    }

    /// The echo keywords declare the port's echo: none by default, the
    /// transport's default grace with `mstp_echo` alone, or the grace given.
    #[test]
    fn echo_keywords_declare_the_ports_echo() {
        let cases = [
            (false, None, Echo::None),
            (true, None, Echo::full()),
            (
                true,
                Some(25),
                Echo::Full {
                    grace: Duration::from_millis(25),
                },
            ),
        ];
        for (echo, echo_grace_ms, expected) in cases {
            let options = MstpOptions {
                echo,
                echo_grace_ms,
                ..options()
            };
            assert_eq!(options.port_echo().unwrap(), expected, "{options:?}");
        }
    }

    /// A zero grace, or any grace without the echo, is refused before the
    /// port opens.
    #[test]
    fn echo_keywords_are_checked_before_open() {
        Python::initialize();
        let cases = [
            (true, Some(0), ZERO_ECHO_GRACE_ERROR),
            (false, Some(0), ECHO_GRACE_WITHOUT_ECHO_ERROR),
            (false, Some(25), ECHO_GRACE_WITHOUT_ECHO_ERROR),
        ];
        for (echo, echo_grace_ms, message) in cases {
            let options = MstpOptions {
                echo,
                echo_grace_ms,
                ..options()
            };
            assert_py_error(build_mstp_transport(&options), message, true);
        }
    }

    /// The port the transport is built on declares the echo the keywords
    /// ask for. Linux only: it opens a pseudo-terminal by its path, which
    /// the serial backend's baud-rate ioctl refuses on macOS.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_opened_port_declares_the_echo() {
        use bacnet_transport::mstp::SerialPort;

        Python::initialize();
        let (fds, path) = Python::attach(|py| -> PyResult<((i32, i32), String)> {
            let os = py.import("os")?;
            let fds: (i32, i32) = os.call_method0("openpty")?.extract()?;
            let path: String = os.call_method1("ttyname", (fds.1,))?.extract()?;
            Ok((fds, path))
        })
        .unwrap();
        // The serial stream registers with the reactor of the runtime it
        // opens in.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _context = runtime.enter();
        let cases = [
            options(),
            MstpOptions {
                echo: true,
                ..options()
            },
            MstpOptions {
                echo: true,
                echo_grace_ms: Some(25),
                ..options()
            },
        ];
        for options in cases {
            let echo = options.port_echo().unwrap();
            let serial = open_serial(&path, options.baud, echo).unwrap();
            assert_eq!(serial.echo(), echo, "{options:?}");
        }
        Python::attach(|py| -> PyResult<()> {
            let os = py.import("os")?;
            os.call_method1("close", (fds.0,))?;
            os.call_method1("close", (fds.1,))?;
            Ok(())
        })
        .unwrap();
    }
}
