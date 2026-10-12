//! Helpers the integration tests share.

#![allow(dead_code)]

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{Context, RegistrationLimits, Tables, Target};
use bacnet_bip_core::{Duration, Instant};

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
/// Its subnet's broadcast address.
pub const LOCAL_BROADCAST: SocketAddrV4 = addr(192, 168, 1, 255, PORT);

/// Small tables, as firmware would size them.
pub type Small = Tables<8, 8>;

/// Storage for one decision over [`Small`].
pub fn storage() -> [Target; Small::MAX_TARGETS] {
    [Target::UNUSED; Small::MAX_TARGETS]
}

/// Tables for [`OWN`] with `bdt` set.
pub fn tables(bdt: &[BdtEntry]) -> Small {
    let mut tables = Small::new(OWN);
    tables.bdt.set(BdtEntries::from_entries(bdt)).unwrap();
    tables
}

/// Limits that admit anything with a nonzero time-to-live.
pub const OPEN: RegistrationLimits = RegistrationLimits {
    min_ttl: 1,
    max_ttl: u16::MAX,
    max_entries_per_source: usize::MAX,
    reserved_capacity: 0,
};

/// Register each of `devices` for `ttl` seconds at `now`, which must succeed.
pub fn register_all(tables: &mut Small, devices: &[SocketAddrV4], ttl: u16, now: Instant) {
    for device in devices {
        let outcome = tables
            .fdt
            .register(*device, ttl, now, &OPEN, false, || true);
        assert!(outcome.result.is_ok(), "{device}: {outcome:?}");
    }
}

/// A decision context at `now` with the default fan-out budget.
pub fn context(now: Instant) -> Context {
    Context {
        now,
        local_broadcast: LOCAL_BROADCAST,
        fdt_fanout: 32,
    }
}

/// `secs` seconds.
pub const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

/// The destinations of `targets`.
pub fn destinations(targets: &[Target]) -> Vec<SocketAddrV4> {
    targets.iter().map(|target| target.destination).collect()
}
