//! Explicit command-line values override profile values, including defaults.

use super::Profile;
use crate::args::Cli;
use std::net::{Ipv4Addr, SocketAddrV4};

/// Only values with clap's CommandLine provenance are present.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Overrides {
    interface: Option<Ipv4Addr>,
    port: Option<u16>,
    broadcast: Option<Ipv4Addr>,
    timeout_ms: Option<u64>,
    min_interval_ms: Option<u64>,
    bbmd: Option<SocketAddrV4>,
    ttl: Option<u16>,
    renewal_secs: Option<u64>,
}

impl Overrides {
    pub(crate) fn from_cli(cli: &Cli) -> Result<Self, String> {
        let has = |key: &str| cli.explicit_args.iter().any(|arg| arg == key);
        if [
            "sc",
            "sc_url",
            "sc_ca",
            "sc_cert",
            "sc_key",
            "sc_vmac",
            "sc_device_uuid",
            "ipv6",
            "ipv6_interface",
            "device_instance",
        ]
        .iter()
        .any(|key| has(key))
        {
            return Err(
                "named profiles support BACnet/IP only; remove SC/IPv6 flags or use --no-profile"
                    .into(),
            );
        }
        Ok(Self {
            interface: cli.interface.filter(|_| has("interface")),
            port: has("port").then_some(cli.port),
            broadcast: has("broadcast").then_some(cli.broadcast),
            timeout_ms: has("timeout").then_some(cli.timeout),
            min_interval_ms: has("min_interval_ms").then_some(cli.min_interval_ms),
            bbmd: cli.foreign_bbmd.filter(|_| has("foreign_bbmd")),
            ttl: has("ttl").then_some(cli.ttl),
            renewal_secs: cli.renewal.filter(|_| has("renewal")),
        })
    }

    /// Apply the explicit flags, then validate the actual connection settings.
    pub(crate) fn resolve(&self, profile: &Profile) -> Result<Profile, String> {
        profile.validate()?;
        let mut resolved = profile.clone();
        let b = &mut resolved.bip;
        if let Some(v) = self.interface {
            b.interface = v;
        }
        if let Some(v) = self.port {
            b.port = v;
        }
        if let Some(v) = self.broadcast {
            b.broadcast = v;
        }
        if let Some(v) = self.timeout_ms {
            b.timeout_ms = v;
        }
        if let Some(v) = self.min_interval_ms {
            b.min_interval_ms = v;
        }
        if let Some(v) = self.bbmd {
            b.bbmd = Some(v);
        }
        if let Some(v) = self.ttl {
            b.ttl = v;
        }
        if let Some(v) = self.renewal_secs {
            b.renewal_secs = Some(v);
        }
        resolved.validate()?;
        Ok(resolved)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Explicit network flags retain the existing nonpersistent entry path.
pub(crate) fn has_transport_flags(cli: &Cli) -> bool {
    cli.explicit_args.iter().any(|id| {
        matches!(
            id.as_str(),
            "interface"
                | "port"
                | "broadcast"
                | "timeout"
                | "min_interval_ms"
                | "foreign_bbmd"
                | "ttl"
                | "renewal"
                | "ipv6"
                | "ipv6_interface"
                | "device_instance"
                | "sc"
                | "sc_url"
                | "sc_ca"
                | "sc_cert"
                | "sc_key"
                | "sc_vmac"
                | "sc_device_uuid"
        )
    })
}
