use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use crate::error::RuntimeError;
use crate::RuntimeTransport;

/// Durable identity of one BACnet network attachment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttachmentId([u8; 16]);

impl AttachmentId {
    /// Creates an attachment identifier from its UUID bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the UUID bytes without applying a transport-specific encoding.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl From<u128> for AttachmentId {
    fn from(value: u128) -> Self {
        Self(value.to_be_bytes())
    }
}

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11],
            b[12], b[13], b[14], b[15]
        )
    }
}

/// BACnet/IP attachment configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BipConfig {
    /// Local IPv4 interface or address selector.
    pub interface: String,
    /// UDP port, including non-default BACnet ports.
    pub port: u16,
    /// Optional broadcast address override.
    pub broadcast: Option<String>,
    /// Optional BBMD IPv4 socket address used for foreign-device registration.
    ///
    /// This must be paired with `foreign_device_ttl`.
    pub bbmd_address: Option<String>,
    /// Optional foreign-device registration lifetime in seconds.
    ///
    /// This must be non-zero and paired with `bbmd_address`.
    pub foreign_device_ttl: Option<u16>,
}

/// MS/TP attachment configuration. Opening the serial device belongs to its factory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MstpConfig {
    /// Stable serial device path.
    pub device: String,
    /// Bus baud rate.
    pub baud: u32,
    /// Local master MAC.
    pub mac: u8,
    /// Highest master address participating in token passing.
    pub max_master: u8,
    /// Maximum frames sent per token hold.
    pub max_info_frames: u8,
}

/// BACnet/SC node configuration. Secret material is referenced, never embedded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScConfig {
    /// Primary secure-connect hub URI.
    pub primary_hub: String,
    /// Ordered failover hub URIs.
    pub failover_hubs: Vec<String>,
    /// Stable six-byte virtual MAC for this node.
    pub local_vmac: [u8; 6],
    /// Optional PEM CA bundle path; native roots are used when absent.
    pub ca_cert: Option<String>,
    /// Optional PEM client certificate chain path for mutual TLS.
    pub client_cert: Option<String>,
    /// Optional PEM client private-key path for mutual TLS.
    pub client_key: Option<String>,
    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: u64,
    /// Heartbeat acknowledgement timeout in milliseconds.
    pub heartbeat_timeout_ms: u64,
    /// Initial reconnect backoff in milliseconds.
    pub reconnect_initial_delay_ms: u64,
    /// Maximum reconnect backoff in milliseconds.
    pub reconnect_max_delay_ms: u64,
    /// Reconnect attempts before switching hubs.
    pub reconnect_max_retries: u32,
}

/// Transport-specific configuration for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportConfig {
    /// BACnet/IP.
    Bip(BipConfig),
    /// BACnet MS/TP.
    Mstp(MstpConfig),
    /// BACnet Secure Connect.
    Sc(ScConfig),
}

/// One independently supervised BACnet attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentConfig {
    /// Durable attachment identity.
    pub id: AttachmentId,
    /// Operator-facing label.
    pub label: String,
    /// Transport configuration.
    pub transport: TransportConfig,
}

/// Top-level runtime configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    /// Initial attachment order; order is used for deterministic device selection.
    pub attachments: Vec<AttachmentConfig>,
    /// Maximum retained runtime events.
    pub event_capacity: usize,
    /// Maximum events returned by one batch.
    pub max_event_batch: usize,
    /// Capacity of each attachment's native COV notification receiver.
    pub cov_channel_capacity: usize,
    /// Grace period for supervised tasks during shutdown.
    pub shutdown_timeout: Duration,
    /// Maximum queued read/write operations.
    pub scheduler_capacity: usize,
    /// Foreground dispatches allowed before one waiting lower lane is served.
    pub max_foreground_burst: usize,
    /// Maximum concurrently executing scheduled operations.
    pub max_inflight_operations: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            attachments: Vec::new(),
            event_capacity: 4096,
            max_event_batch: 256,
            cov_channel_capacity: bacnet_client::client::DEFAULT_COV_CHANNEL_CAPACITY,
            shutdown_timeout: Duration::from_secs(5),
            scheduler_capacity: 1024,
            max_foreground_burst: 8,
            max_inflight_operations: 32,
        }
    }
}

impl RuntimeConfig {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if self.event_capacity == 0 {
            return Err(RuntimeError::invalid_config(
                "event_capacity must be non-zero",
            ));
        }
        if self.max_event_batch == 0 || self.max_event_batch > self.event_capacity {
            return Err(RuntimeError::invalid_config(
                "max_event_batch must be in 1..=event_capacity",
            ));
        }
        if !(1..=bacnet_client::client::MAX_COV_CHANNEL_CAPACITY)
            .contains(&self.cov_channel_capacity)
        {
            return Err(RuntimeError::invalid_config(format!(
                "cov_channel_capacity must be in 1..={}",
                bacnet_client::client::MAX_COV_CHANNEL_CAPACITY
            )));
        }
        if self.shutdown_timeout.is_zero() {
            return Err(RuntimeError::invalid_config(
                "shutdown_timeout must be non-zero",
            ));
        }
        if self.scheduler_capacity == 0
            || self.max_foreground_burst == 0
            || self.max_inflight_operations == 0
        {
            return Err(RuntimeError::invalid_config(
                "scheduler_capacity, max_foreground_burst, and max_inflight_operations must be non-zero",
            ));
        }

        validate_attachments(&self.attachments)
    }
}

pub(crate) fn validate_attachments(attachments: &[AttachmentConfig]) -> Result<(), RuntimeError> {
    let mut ids = HashSet::with_capacity(attachments.len());
    for attachment in attachments {
        if !ids.insert(attachment.id) {
            return Err(RuntimeError::invalid_config(format!(
                "duplicate attachment id {}",
                attachment.id
            )));
        }
        if attachment.label.trim().is_empty() {
            return Err(RuntimeError::invalid_config(format!(
                "attachment {} has an empty label",
                attachment.id
            )));
        }
        RuntimeTransport::validate_config(attachment)?;
    }
    Ok(())
}
