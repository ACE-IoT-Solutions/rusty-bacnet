//! Helpers the integration tests share.

#![allow(dead_code)]

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::BdtEntry;
use bacnet_bip_core::bbmd::RegistrationLimits;
use bacnet_bip_core::Duration;

/// The default B/IP port.
pub const PORT: u16 = 0xBAC0;
/// A subnet mask of /24.
pub const MASK_24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
/// The two-hop mask.
pub const MASK_32: Ipv4Addr = Ipv4Addr::BROADCAST;

/// `a.b.c.d:port`.
pub const fn addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// A row for `address` with `mask`.
pub const fn row(address: SocketAddrV4, mask: Ipv4Addr) -> BdtEntry {
    BdtEntry::new(address, mask)
}

/// The BBMD under test: 192.168.1.1, at the default port.
pub const OWN: SocketAddrV4 = addr(192, 168, 1, 1, PORT);

/// Limits that admit anything with a nonzero time-to-live.
pub const OPEN: RegistrationLimits = RegistrationLimits {
    min_ttl: 1,
    max_ttl: u16::MAX,
    max_entries_per_source: usize::MAX,
    reserved_capacity: 0,
};

/// `secs` seconds.
pub const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}
