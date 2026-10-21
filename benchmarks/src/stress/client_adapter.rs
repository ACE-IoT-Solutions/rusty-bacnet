//! Current-version setup and typed error conversion for the common driver.
use super::client_diagnostics::FailureKind;
pub use super::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use bacnet_types::error::Error;

pub fn classify(error: &Error) -> FailureKind {
    match error {
        Error::Timeout(_) => FailureKind::Timeout,
        Error::Abort { reason } => FailureKind::Abort {
            reason: reason.to_raw(),
        },
        Error::Reject { reason } => FailureKind::Reject {
            reason: reason.to_raw(),
        },
        Error::Protocol { class, code } | Error::Structured { class, code, .. } => {
            FailureKind::Protocol {
                class: u32::from(class.to_raw()),
                code: u32::from(code.to_raw()),
            }
        }
        Error::Transport(error) => FailureKind::Transport {
            io_kind: format!("{:?}", error.kind()),
        },
        _ => FailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
    #[test]
    fn error_variants_preserve_numeric_codes_without_display_parsing() {
        let cases = [
            (
                Error::Timeout(std::time::Duration::from_secs(5)),
                FailureKind::Timeout,
            ),
            (
                Error::Abort {
                    reason: AbortReason::from_raw(1),
                },
                FailureKind::Abort { reason: 1 },
            ),
            (
                Error::Abort {
                    reason: AbortReason::from_raw(65),
                },
                FailureKind::Abort { reason: 65 },
            ),
            (
                Error::Reject {
                    reason: RejectReason::from_raw(9),
                },
                FailureKind::Reject { reason: 9 },
            ),
            (
                Error::Protocol {
                    class: ErrorClass::from_raw(2),
                    code: ErrorCode::from_raw(32),
                },
                FailureKind::Protocol { class: 2, code: 32 },
            ),
            (
                Error::Transport(std::io::ErrorKind::ConnectionRefused.into()),
                FailureKind::Transport {
                    io_kind: "ConnectionRefused".into(),
                },
            ),
            (Error::Encoding("fixture".into()), FailureKind::Other),
        ];
        for (error, expected) in cases {
            assert_eq!(classify(&error), expected);
        }
    }
}
