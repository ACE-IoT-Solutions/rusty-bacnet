use crate::{AttachmentId, DeviceKey};

/// Stable machine-readable runtime error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Configuration is invalid.
    InvalidConfig,
    /// A requested reconcile revision is stale.
    StaleRevision,
    /// The requested attachment does not exist.
    AttachmentNotFound,
    /// The attachment transport is unsupported by this build.
    UnsupportedTransport,
    /// A local socket address is already occupied.
    BindConflict,
    /// A BBMD rejected foreign-device registration.
    ForeignDeviceRegistrationFailed,
    /// A serial device is unavailable.
    SerialUnavailable,
    /// Secure-connect TLS configuration is invalid.
    TlsConfig,
    /// Secure-connect peer authentication failed.
    TlsAuth,
    /// Secure-connect is disconnected.
    ScDisconnected,
    /// A BACnet operation timed out.
    Timeout,
    /// A BACnet reject was received.
    Reject,
    /// A BACnet abort was received.
    Abort,
    /// A BACnet error response was received.
    BacnetError,
    /// A wire value could not be decoded.
    Decode,
    /// No route is available.
    RouteUnavailable,
    /// A device is unavailable.
    DeviceUnavailable,
    /// One property failed.
    PropertyError,
    /// A bounded queue rejected work.
    Backpressure,
    /// The operation was cancelled.
    Cancelled,
    /// The operation expired before completion.
    DeadlineExceeded,
    /// Runtime events were lost to bounded-buffer pressure.
    EventLagged,
    /// A transactional replacement could not restore prior state.
    RollbackFailed,
    /// An invariant or unexpected internal operation failed.
    Internal,
}

/// Structured runtime failure suitable for stable language bindings.
#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
#[error("{code:?}: {message}")]
pub struct RuntimeError {
    /// Machine-readable category.
    pub code: ErrorCode,
    /// Whether retrying without a configuration change may succeed.
    pub retryable: bool,
    /// Optional attachment context.
    pub attachment_id: Option<AttachmentId>,
    /// Human-readable detail; callers must branch on `code`, not this value.
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

    pub(crate) fn unsupported_transport(attachment_id: AttachmentId, transport: &str) -> Self {
        Self {
            code: ErrorCode::UnsupportedTransport,
            retryable: false,
            attachment_id: Some(attachment_id),
            message: format!("{transport} attachment factory is not enabled yet"),
        }
    }

    pub(crate) fn invalid_attachment_config(
        attachment_id: AttachmentId,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: ErrorCode::InvalidConfig,
            retryable: false,
            attachment_id: Some(attachment_id),
            message: message.into(),
        }
    }

    pub(crate) fn attachment_start(
        attachment_id: AttachmentId,
        error: bacnet_types::error::Error,
    ) -> Self {
        let (code, retryable) = match &error {
            bacnet_types::error::Error::Transport(io)
                if io.kind() == std::io::ErrorKind::AddrInUse =>
            {
                (ErrorCode::BindConflict, true)
            }
            bacnet_types::error::Error::Transport(_) => (ErrorCode::DeviceUnavailable, true),
            bacnet_types::error::Error::Timeout(_) => (ErrorCode::Timeout, true),
            bacnet_types::error::Error::Decoding { .. } => (ErrorCode::Decode, false),
            bacnet_types::error::Error::Bvlc { result_code }
                if *result_code
                    == bacnet_types::enums::BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK =>
            {
                (ErrorCode::ForeignDeviceRegistrationFailed, true)
            }
            _ => (ErrorCode::Internal, false),
        };
        Self {
            code,
            retryable,
            attachment_id: Some(attachment_id),
            message: error.to_string(),
        }
    }

    pub(crate) fn serial_unavailable(
        attachment_id: AttachmentId,
        error: bacnet_types::error::Error,
    ) -> Self {
        Self {
            code: ErrorCode::SerialUnavailable,
            retryable: true,
            attachment_id: Some(attachment_id),
            message: error.to_string(),
        }
    }

    pub(crate) fn tls_config(
        attachment_id: AttachmentId,
        error: bacnet_types::error::Error,
    ) -> Self {
        Self {
            code: ErrorCode::TlsConfig,
            retryable: false,
            attachment_id: Some(attachment_id),
            message: error.to_string(),
        }
    }

    pub(crate) fn sc_connect(
        attachment_id: AttachmentId,
        error: bacnet_types::error::Error,
    ) -> Self {
        let code = match &error {
            bacnet_types::error::Error::Timeout(_) => ErrorCode::ScDisconnected,
            bacnet_types::error::Error::Transport(_) => ErrorCode::ScDisconnected,
            _ => ErrorCode::TlsAuth,
        };
        Self {
            code,
            retryable: code == ErrorCode::ScDisconnected,
            attachment_id: Some(attachment_id),
            message: error.to_string(),
        }
    }

    pub(crate) fn stale_revision(revision: u64, current: u64) -> Self {
        Self {
            code: ErrorCode::StaleRevision,
            retryable: false,
            attachment_id: None,
            message: format!("reconcile revision {revision} is not newer than {current}"),
        }
    }

    pub(crate) fn rollback_failed(
        attachment_id: AttachmentId,
        original: &RuntimeError,
        rollback: &RuntimeError,
    ) -> Self {
        Self {
            code: ErrorCode::RollbackFailed,
            retryable: false,
            attachment_id: Some(attachment_id),
            message: format!(
                "replacement failed ({original}); restoring prior attachment failed ({rollback})"
            ),
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

    pub(crate) fn operation(
        attachment_id: AttachmentId,
        error: bacnet_types::error::Error,
    ) -> Self {
        let (code, retryable) = match &error {
            bacnet_types::error::Error::Protocol { .. } => (ErrorCode::BacnetError, false),
            bacnet_types::error::Error::Reject { .. } => (ErrorCode::Reject, false),
            bacnet_types::error::Error::Abort { .. } => (ErrorCode::Abort, true),
            bacnet_types::error::Error::Timeout(_) => (ErrorCode::Timeout, true),
            bacnet_types::error::Error::Decoding { .. } => (ErrorCode::Decode, false),
            bacnet_types::error::Error::Transport(_) => (ErrorCode::DeviceUnavailable, true),
            _ => (ErrorCode::Internal, false),
        };
        Self {
            code,
            retryable,
            attachment_id: Some(attachment_id),
            message: error.to_string(),
        }
    }

    pub(crate) fn attachment_not_found(attachment_id: AttachmentId) -> Self {
        Self {
            code: ErrorCode::AttachmentNotFound,
            retryable: false,
            attachment_id: Some(attachment_id),
            message: format!("attachment {attachment_id} is not configured"),
        }
    }

    pub(crate) fn device_not_found(device: DeviceKey) -> Self {
        Self {
            code: ErrorCode::DeviceUnavailable,
            retryable: true,
            attachment_id: Some(device.attachment_id),
            message: format!(
                "device {} has no observation on attachment {}",
                device.device_instance, device.attachment_id
            ),
        }
    }

    pub(crate) fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::DeadlineExceeded,
            retryable: true,
            attachment_id: None,
            message: message.into(),
        }
    }

    pub(crate) fn property_error(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PropertyError,
            retryable: false,
            attachment_id: None,
            message: message.into(),
        }
    }

    pub(crate) fn decode(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Decode,
            retryable: false,
            attachment_id: None,
            message: message.into(),
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

    pub(crate) fn cancelled(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Cancelled,
            retryable: false,
            attachment_id: None,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, RuntimeError};
    use crate::AttachmentId;
    use bacnet_types::enums::BvlcResultCode;

    #[test]
    fn foreign_device_nak_has_stable_retryable_attachment_error() {
        let attachment_id = AttachmentId::from(7);
        let error = RuntimeError::attachment_start(
            attachment_id,
            bacnet_types::error::Error::Bvlc {
                result_code: BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            },
        );

        assert_eq!(error.code, ErrorCode::ForeignDeviceRegistrationFailed);
        assert!(error.retryable);
        assert_eq!(error.attachment_id, Some(attachment_id));
    }
}
