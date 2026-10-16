use std::collections::HashSet;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use crate::error::RuntimeError;

/// Durable identity of one BACnet network attachment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttachmentId([u8; 16]);

impl AttachmentId {
    /// Creates an identifier from UUID bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the UUID bytes.
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
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

/// BACnet/IP attachment configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BipConfig {
    /// Local IPv4 interface or address selector.
    pub interface: String,
    /// UDP port. Zero requests an ephemeral port.
    pub port: u16,
    /// Optional directed broadcast address override.
    pub broadcast: Option<String>,
    /// Optional BBMD address for foreign-device registration.
    pub bbmd_address: Option<String>,
    /// Foreign-device registration lifetime in seconds.
    pub foreign_device_ttl: Option<u16>,
}

impl Default for BipConfig {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED.to_string(),
            port: bacnet_transport::bip::DEFAULT_BACNET_PORT,
            broadcast: None,
            bbmd_address: None,
            foreign_device_ttl: None,
        }
    }
}

/// MS/TP configuration retained only so unsupported transport requests fail
/// explicitly at validation time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MstpConfig {
    /// Serial device path.
    pub device: String,
    /// Bus baud rate.
    pub baud: u32,
    /// Local station address.
    pub mac: u8,
    /// Highest master address.
    pub max_master: u8,
    /// Maximum frames per token hold.
    pub max_info_frames: u8,
}

/// BACnet/SC configuration retained only so unsupported transport requests
/// fail explicitly at validation time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScConfig {
    /// Primary hub URI.
    pub primary_hub: String,
    /// Failover hub URIs.
    pub failover_hubs: Vec<String>,
    /// Local VMAC.
    pub local_vmac: [u8; 6],
    /// Device UUID.
    pub device_uuid: [u8; 16],
    /// Optional CA bundle path (unsupported until SC transport ownership lands).
    pub ca_cert: Option<String>,
    /// Optional client certificate path.
    pub client_cert: Option<String>,
    /// Optional client key path.
    pub client_key: Option<String>,
    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: u64,
    /// Heartbeat timeout in milliseconds.
    pub heartbeat_timeout_ms: u64,
    /// Initial reconnect delay in milliseconds.
    pub reconnect_initial_delay_ms: u64,
    /// Maximum reconnect delay in milliseconds.
    pub reconnect_max_delay_ms: u64,
    /// Reconnect attempts before switching hubs.
    pub reconnect_max_retries: u32,
    /// Whether reconnect attempts are unbounded.
    pub reconnect_forever: bool,
}

/// Transport configuration for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportConfig {
    /// BACnet/IP, the only transport implemented by this runtime slice.
    Bip(BipConfig),
    /// MS/TP, rejected by this runtime slice.
    Mstp(MstpConfig),
    /// BACnet/SC, rejected by this runtime slice.
    Sc(ScConfig),
}

/// One independently supervised BACnet attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentConfig {
    /// Durable identity.
    pub id: AttachmentId,
    /// Operator-facing label.
    pub label: String,
    /// Transport configuration.
    pub transport: TransportConfig,
}

/// Runtime-wide bounded scheduling and lifecycle configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    /// Initial attachment configuration.
    pub attachments: Vec<AttachmentConfig>,
    /// Maximum time allowed for cooperative shutdown.
    pub shutdown_timeout: Duration,
    /// Maximum retained runtime events (reserved for the event subsystem).
    pub event_capacity: usize,
    /// Maximum events returned by one batch (reserved for the event subsystem).
    pub max_event_batch: usize,
    /// COV receiver capacity (reserved until COV supervision lands).
    pub cov_channel_capacity: usize,
    /// Maximum queued scheduler work (the B/IP slice rejects when slots are full).
    pub scheduler_capacity: usize,
    /// Foreground scheduler fairness burst (reserved for the scheduler subsystem).
    pub max_foreground_burst: usize,
    /// Maximum simultaneously executing read/write batches.
    pub max_inflight_operations: usize,
    /// Maximum items accepted in one ordered batch.
    pub max_batch_items: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            attachments: Vec::new(),
            shutdown_timeout: Duration::from_secs(5),
            event_capacity: 4096,
            max_event_batch: 256,
            cov_channel_capacity: 64,
            scheduler_capacity: 1024,
            max_foreground_burst: 8,
            max_inflight_operations: 32,
            max_batch_items: 1024,
        }
    }
}

impl RuntimeConfig {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if self.shutdown_timeout.is_zero() {
            return Err(RuntimeError::invalid_config(
                "shutdown_timeout must be non-zero",
            ));
        }
        if self.event_capacity == 0
            || self.max_event_batch == 0
            || self.max_event_batch > self.event_capacity
            || self.cov_channel_capacity == 0
            || self.scheduler_capacity == 0
            || self.max_foreground_burst == 0
        {
            return Err(RuntimeError::invalid_config(
                "event and scheduler capacities must be non-zero, with max_event_batch <= event_capacity",
            ));
        }
        if self.max_inflight_operations == 0 {
            return Err(RuntimeError::invalid_config(
                "max_inflight_operations must be non-zero",
            ));
        }
        if self.max_batch_items == 0 {
            return Err(RuntimeError::invalid_config(
                "max_batch_items must be non-zero",
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
            return Err(RuntimeError::attachment_config(
                attachment.id,
                "attachment label must not be empty",
            ));
        }
        match &attachment.transport {
            TransportConfig::Bip(config) => validate_bip(attachment.id, config)?,
            TransportConfig::Mstp(_) => {
                return Err(RuntimeError::unsupported(attachment.id, "MS/TP"));
            }
            TransportConfig::Sc(_) => {
                return Err(RuntimeError::unsupported(attachment.id, "BACnet/SC"));
            }
        }
    }
    Ok(())
}

fn validate_bip(id: AttachmentId, config: &BipConfig) -> Result<(), RuntimeError> {
    config.interface.parse::<Ipv4Addr>().map_err(|error| {
        RuntimeError::attachment_config(id, format!("invalid B/IP interface: {error}"))
    })?;
    config
        .broadcast
        .as_deref()
        .unwrap_or("255.255.255.255")
        .parse::<Ipv4Addr>()
        .map_err(|error| {
            RuntimeError::attachment_config(id, format!("invalid B/IP broadcast: {error}"))
        })?;

    match (&config.bbmd_address, config.foreign_device_ttl) {
        (None, None) => Ok(()),
        (Some(address), Some(ttl)) => {
            let address = address.parse::<SocketAddrV4>().map_err(|error| {
                RuntimeError::attachment_config(id, format!("invalid BBMD address: {error}"))
            })?;
            if address.port() == 0 {
                return Err(RuntimeError::attachment_config(
                    id,
                    "BBMD port must be non-zero",
                ));
            }
            if ttl == 0 {
                return Err(RuntimeError::attachment_config(
                    id,
                    "foreign-device TTL must be non-zero",
                ));
            }
            Ok(())
        }
        _ => Err(RuntimeError::attachment_config(
            id,
            "BBMD address and foreign-device TTL must be provided together",
        )),
    }
}
