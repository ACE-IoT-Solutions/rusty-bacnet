use crate::AttachmentId;

/// Stable machine-readable runtime error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Configuration is invalid.
    InvalidConfig,
    /// The requested attachment is not configured.
    AttachmentNotFound,
    /// The requested transport is not implemented by this runtime slice.
    UnsupportedTransport,
    /// A local UDP bind failed because the address is unavailable.
    BindConflict,
    /// A BBMD rejected or lost foreign-device registration.
    ForeignDeviceRegistrationFailed,
    /// A BACnet operation timed out.
    Timeout,
    /// A BACnet peer or transport is unavailable.
    DeviceUnavailable,
    /// A BACnet service returned an error, reject or abort.
    BacnetError,
    /// The bounded operation scheduler has no available slot.
    Backpressure,
    /// The runtime or operation was cancelled.
    Cancelled,
    /// An operation exceeded its configured deadline.
    DeadlineExceeded,
    /// A reconcile revision is not newer than the applied revision.
    StaleRevision,
    /// A replacement failed and the prior attachment could not be restored.
    RollbackFailed,
    /// An unexpected internal failure occurred.
    Internal,
}

/// Structured runtime failure suitable for language bindings.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct RuntimeError {
    /// Machine-readable category.
    pub code: ErrorCode,
    /// Whether retrying without changing configuration may succeed.
    pub retryable: bool,
    /// Attachment context, when applicable.
    pub attachment_id: Option<AttachmentId>,
    /// Human-readable diagnostic detail.
    pub message: String,
}

impl RuntimeError {
    pub(crate) fn invalid_config(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidConfig,
            retryable: false,
            attachment_id: None,
            message: message.into(),
        }
    }

    pub(crate) fn attachment_config(id: AttachmentId, message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidConfig,
            retryable: false,
            attachment_id: Some(id),
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(id: AttachmentId, transport: &str) -> Self {
        Self {
            code: ErrorCode::UnsupportedTransport,
            retryable: false,
            attachment_id: Some(id),
            message: format!("{transport} runtime support is not available in this release"),
        }
    }

    pub(crate) fn attachment_not_found(id: AttachmentId) -> Self {
        Self {
            code: ErrorCode::AttachmentNotFound,
            retryable: false,
            attachment_id: Some(id),
            message: format!("attachment {id} is not configured"),
        }
    }

    pub(crate) fn start(id: AttachmentId, error: bacnet_types::error::Error) -> Self {
        let (code, retryable) = match &error {
            bacnet_types::error::Error::Transport(io) if is_bind_conflict(io) => {
                (ErrorCode::BindConflict, true)
            }
            bacnet_types::error::Error::Transport(_) => (ErrorCode::DeviceUnavailable, true),
            bacnet_types::error::Error::Timeout(_) => (ErrorCode::Timeout, true),
            _ => (ErrorCode::Internal, false),
        };
        Self {
            code,
            retryable,
            attachment_id: Some(id),
            message: error.to_string(),
        }
    }

    pub(crate) fn operation(id: AttachmentId, error: bacnet_types::error::Error) -> Self {
        use bacnet_types::error::RemoteError;

        let (code, retryable) = match &error {
            bacnet_types::error::Error::Remote(RemoteError::Abort { .. }) => {
                (ErrorCode::BacnetError, true)
            }
            bacnet_types::error::Error::Remote(_)
            | bacnet_types::error::Error::Structured { .. } => (ErrorCode::BacnetError, false),
            bacnet_types::error::Error::Timeout(_) => (ErrorCode::Timeout, true),
            bacnet_types::error::Error::Transport(_) => (ErrorCode::DeviceUnavailable, true),
            _ => (ErrorCode::Internal, false),
        };
        Self {
            code,
            retryable,
            attachment_id: Some(id),
            message: error.to_string(),
        }
    }

    pub(crate) fn stopped() -> Self {
        Self {
            code: ErrorCode::Cancelled,
            retryable: false,
            attachment_id: None,
            message: "runtime is stopped".to_owned(),
        }
    }

    pub(crate) fn deadline() -> Self {
        Self {
            code: ErrorCode::DeadlineExceeded,
            retryable: true,
            attachment_id: None,
            message: "batch deadline exceeded".to_owned(),
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Internal,
            retryable: false,
            attachment_id: None,
            message: message.into(),
        }
    }
}

fn is_bind_conflict(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::AddrInUse
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(10_013 | 10_048)))
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, RuntimeError, is_bind_conflict};

    #[test]
    fn remote_responses_keep_runtime_categories_and_retry_policy() {
        use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode as BacnetCode, RejectReason};
        use bacnet_types::error::{Error, RemoteError};

        let id = crate::AttachmentId::from(1);
        for (remote, retryable) in [
            (
                RemoteError::Error {
                    class: ErrorClass::PROPERTY,
                    code: BacnetCode::UNKNOWN_PROPERTY,
                },
                false,
            ),
            (
                RemoteError::Reject {
                    reason: RejectReason::OTHER,
                },
                false,
            ),
            (
                RemoteError::Abort {
                    reason: AbortReason::OTHER,
                },
                true,
            ),
        ] {
            let error = Error::Remote(remote);
            let message = error.to_string();
            let mapped = RuntimeError::operation(id, error);
            assert_eq!(mapped.code, ErrorCode::BacnetError);
            assert_eq!(mapped.retryable, retryable);
            assert_eq!(mapped.attachment_id, Some(id));
            assert_eq!(mapped.message, message);
        }
    }

    #[test]
    fn address_in_use_is_a_bind_conflict() {
        let error = std::io::Error::new(std::io::ErrorKind::AddrInUse, "occupied");
        assert!(is_bind_conflict(&error));
    }

    #[cfg(windows)]
    #[test]
    fn winsock_exclusive_bind_errors_are_bind_conflicts() {
        assert!(is_bind_conflict(&std::io::Error::from_raw_os_error(10_013)));
        assert!(is_bind_conflict(&std::io::Error::from_raw_os_error(10_048)));
    }
}
