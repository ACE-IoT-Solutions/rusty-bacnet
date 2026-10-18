//! Versioned, private B/IP connection profiles. Resolution never performs I/O.

pub(crate) mod overrides;
pub(crate) mod service;
pub(crate) mod startup;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use crate::transport::TransportArgs;
pub(crate) use overrides::Overrides;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4};

/// Settings persisted for one named connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    /// Schema version; unknown versions must not silently acquire defaults.
    pub(crate) version: u32,
    /// The only supported persisted transport.
    pub(crate) transport: Transport,
    /// Network settings.
    pub(crate) bip: Bip,
    /// Recorded policy. The current TUI always remains read-only.
    pub(crate) safety: Safety,
    /// Explicit first-use authorization, persisted before connecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_at_unix: Option<u64>,
}

/// Profile transport discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Transport {
    Bip,
}

/// B/IP network settings; port zero asks the OS for an ephemeral port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Bip {
    pub(crate) interface: Ipv4Addr,
    pub(crate) port: u16,
    pub(crate) broadcast: Ipv4Addr,
    pub(crate) timeout_ms: u64,
    pub(crate) min_interval_ms: u64,
    pub(crate) bbmd: Option<SocketAddrV4>,
    pub(crate) ttl: u16,
    pub(crate) renewal_secs: Option<u64>,
}

impl Default for Bip {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 47808,
            broadcast: Ipv4Addr::BROADCAST,
            timeout_ms: 6000,
            min_interval_ms: 0,
            bbmd: None,
            ttl: 300,
            renewal_secs: None,
        }
    }
}

/// Whether a profile may arm a write session (`W`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Writes {
    /// Read-only for good: `W` says why and arms nothing.
    Never,
    /// A time-boxed write session may be armed.
    Session,
}

/// A profile's write policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Safety {
    /// Whether a write session may be armed.
    pub(crate) writes: Writes,
    /// Allow dangerous operations (today, commanding priority 6) inside an
    /// armed write session.
    pub(crate) dangerous: bool,
}

impl Default for Safety {
    /// Sessions may be armed, with nothing dangerous. Arming is itself the
    /// explicit, time-boxed step.
    fn default() -> Self {
        Self {
            writes: Writes::Session,
            dangerous: false,
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            version: 1,
            transport: Transport::Bip,
            bip: Bip::default(),
            safety: Safety::default(),
            authorized_at_unix: None,
        }
    }
}

impl Profile {
    /// Deterministic validation before any bind, probe, or registration.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported profile version {}", self.version));
        }
        if self.safety.dangerous && self.safety.writes == Writes::Never {
            return Err("dangerous = true needs writes = \"session\"".into());
        }
        let b = &self.bip;
        if b.interface.is_multicast() || b.interface.is_broadcast() {
            return Err("interface must be a unicast IPv4 address or 0.0.0.0".into());
        }
        if b.min_interval_ms > bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS {
            return Err("min_interval_ms must be at most 3600000".into());
        }
        if let Some(bbmd) = b.bbmd {
            if bbmd.port() == 0
                || bbmd.ip().is_unspecified()
                || bbmd.ip().is_multicast()
                || bbmd.ip().is_broadcast()
                || (*bbmd.ip() == b.broadcast
                    && !bbmd.ip().is_loopback()
                    && *bbmd.ip() != b.interface)
            {
                return Err("BBMD must be a unicast IPv4 endpoint with a nonzero port".into());
            }
            if b.ttl == 0
                || b.renewal_secs
                    .is_some_and(|n| n == 0 || n >= u64::from(b.ttl))
            {
                return Err("foreign-device mode needs TTL > 0 and renewal > 0 and < TTL".into());
            }
        } else if b.renewal_secs.is_some() {
            return Err("renewal requires a BBMD (clear renewal for Normal mode)".into());
        }
        Ok(())
    }

    /// Construct the existing CLI transport configuration after validation.
    pub(crate) fn transport_args(&self) -> Result<TransportArgs, String> {
        self.validate()?;
        let b = &self.bip;
        Ok(TransportArgs {
            interface: b.interface,
            port: b.port,
            broadcast: b.broadcast,
            timeout_ms: b.timeout_ms,
            min_interval_ms: b.min_interval_ms,
            foreign_device: crate::foreign::config(b.bbmd, b.ttl, b.renewal_secs),
            sc: false,
            sc_url: None,
            sc_ca: None,
            sc_cert: None,
            sc_key: None,
            sc_vmac: None,
            sc_device_uuid: None,
            ipv6: false,
            ipv6_interface: None,
            device_instance: None,
        })
    }
}
