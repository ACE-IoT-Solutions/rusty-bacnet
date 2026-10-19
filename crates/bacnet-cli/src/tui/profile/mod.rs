//! Versioned, private connection profiles (B/IP and BACnet/SC). Resolution
//! never performs I/O; [`checks`] does the file checks before connecting.

pub(crate) mod checks;
pub(crate) mod overrides;
pub(crate) mod sc;
#[cfg(test)]
pub(crate) mod sc_tests;
pub(crate) mod service;
pub(crate) mod startup;
pub(crate) mod store;
#[cfg(test)]
pub(crate) mod tests;

use crate::transport::TransportArgs;
use crate::tui::worker::session::TransportKind;
pub(crate) use overrides::Overrides;
pub(crate) use sc::Sc;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4};

/// Settings persisted for one named connection.
///
/// Each transport keeps its settings in its own table, and exactly the one
/// `transport` names is present. MS/TP (#969) adds a [`Transport`] variant
/// and an `mstp` table the same way. Files written before SC support (B/IP
/// only) have the same shape and load unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    /// Schema version; unknown versions must not silently acquire defaults.
    pub(crate) version: u32,
    /// Which transport table applies.
    pub(crate) transport: Transport,
    /// B/IP settings, present for `transport = "bip"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bip: Option<Bip>,
    /// BACnet/SC settings, present for `transport = "sc"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sc: Option<Box<Sc>>,
    /// Recorded policy. The current TUI always remains read-only.
    pub(crate) safety: Safety,
    /// Explicit first-use authorization, persisted before connecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_at_unix: Option<u64>,
    /// The write policy that authorization covered. A policy raised past
    /// it (or a file from before this was recorded) needs authorizing again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_safety: Option<Safety>,
}

/// Profile transport discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Transport {
    /// BACnet/IP.
    Bip,
    /// BACnet/SC.
    Sc,
}

impl Transport {
    /// The worker's client kind.
    pub(crate) fn kind(self) -> TransportKind {
        match self {
            Self::Bip => TransportKind::Bip,
            Self::Sc => TransportKind::Sc,
        }
    }
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

impl Safety {
    /// The read-only policy, which an authorization recorded before write
    /// policies existed is taken to cover.
    pub(crate) const READ_ONLY: Self = Self {
        writes: Writes::Never,
        dangerous: false,
    };

    /// True when `self` allows something `from` did not: writes where
    /// there were none, or dangerous operations.
    pub(crate) fn raises(&self, from: &Self) -> bool {
        (self.writes == Writes::Session && from.writes == Writes::Never)
            || (self.dangerous && !from.dangerous)
    }
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
            bip: Some(Bip::default()),
            sc: None,
            safety: Safety::default(),
            authorized_at_unix: None,
            authorized_safety: None,
        }
    }
}

impl Profile {
    /// A new SC profile with empty settings.
    pub(crate) fn sc_default() -> Self {
        Self {
            transport: Transport::Sc,
            bip: None,
            sc: Some(Box::default()),
            ..Self::default()
        }
    }

    /// Drop an authorization that doesn't cover the current write policy,
    /// so connecting asks for it again before anything can be armed.
    pub(crate) fn revoke_if_raised(&mut self) {
        let covered = self
            .authorized_safety
            .as_ref()
            .unwrap_or(&Safety::READ_ONLY);
        if self.authorized_at_unix.is_some() && self.safety.raises(covered) {
            self.authorized_at_unix = None;
            self.authorized_safety = None;
        }
    }

    /// Deterministic validation before any bind, probe, or registration.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported profile version {}", self.version));
        }
        if self.safety.dangerous && self.safety.writes == Writes::Never {
            return Err("dangerous = true needs writes = \"session\"".into());
        }
        match (self.transport, &self.bip, &self.sc) {
            (Transport::Bip, Some(bip), None) => bip.validate(),
            (Transport::Sc, None, Some(sc)) => sc.validate(),
            (Transport::Bip, ..) => Err("a B/IP profile needs a [bip] table and no other".into()),
            (Transport::Sc, ..) => Err("an SC profile needs an [sc] table and no other".into()),
        }
    }

    /// Construct the existing CLI transport configuration after validation.
    pub(crate) fn transport_args(&self) -> Result<TransportArgs, String> {
        self.validate()?;
        let mut args = TransportArgs {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 0,
            broadcast: Ipv4Addr::BROADCAST,
            timeout_ms: 0,
            min_interval_ms: 0,
            foreign_device: None,
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
        };
        if let Some(b) = &self.bip {
            args.interface = b.interface;
            args.port = b.port;
            args.broadcast = b.broadcast;
            args.timeout_ms = b.timeout_ms;
            args.min_interval_ms = b.min_interval_ms;
            args.foreign_device = crate::foreign::config(b.bbmd, b.ttl, b.renewal_secs);
        }
        if let Some(sc) = &self.sc {
            let (Some(vmac), Some(uuid)) = (sc.vmac, sc.device_uuid) else {
                return Err("the SC profile has no VMAC or device UUID yet; save it first".into());
            };
            args.timeout_ms = sc.timeout_ms;
            args.min_interval_ms = sc.min_interval_ms;
            args.sc = true;
            args.sc_url = Some(sc.hub_url.clone());
            args.sc_ca = Some(sc.ca.clone());
            args.sc_cert = Some(sc.cert.clone());
            args.sc_key = Some(sc.key.clone());
            args.sc_vmac = Some(vmac.0);
            args.sc_device_uuid = Some(uuid.0);
        }
        Ok(args)
    }
}

impl Bip {
    fn validate(&self) -> Result<(), String> {
        let b = self;
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
}

/// Field access for tests written against one transport.
#[cfg(test)]
impl Profile {
    pub(crate) fn bip(&self) -> &Bip {
        self.bip.as_ref().expect("a B/IP profile")
    }
    pub(crate) fn bip_mut(&mut self) -> &mut Bip {
        self.bip.as_mut().expect("a B/IP profile")
    }
    pub(crate) fn sc_mut(&mut self) -> &mut Sc {
        self.sc.as_mut().expect("an SC profile")
    }
}
