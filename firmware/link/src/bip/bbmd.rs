//! The B/IP image's BBMD part: `bacnet-bip-core`'s tables and forwarding
//! (Annex J.4, J.5), in tables of four rows held in the image's static
//! storage.
//!
//! [`run`] sets a BDT with a one-hop and a two-hop peer, registers foreign
//! devices until the table is full, and takes each forwarding decision once:
//! an Original-Broadcast-NPDU from the local subnet, a two-hop peer's
//! Forwarded-NPDU, one from a sender that is no peer, a foreign device's
//! Distribute-Broadcast-To-Network, one from a sender that isn't
//! registered, and, after one registration lapses, the BBMD's own
//! broadcast and the FDT rows a Read-FDT-Ack would carry. Every decision's
//! destinations go into the same storage. It returns a [`Summary`].

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcResultCode, FdtEntry};
use bacnet_bip_core::bbmd::{
    Arrival, Context, Distribution, ForwardedRefusal, RegistrationLimits, RegistrationRefusal,
    Tables, Target, TWO_HOP_MASK,
};
use bacnet_bip_core::{Duration, Instant};

use super::NPDU;

/// The BBMD's tables: four BDT rows and four registrations.
pub type Bbmd = Tables<4, 4>;
/// Destinations one decision can have.
pub const TARGETS: usize = Bbmd::MAX_TARGETS;

const fn address(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// This BBMD.
pub const OWN: SocketAddrV4 = address(192, 168, 1, 10, 0xBAC0);
/// Its subnet's broadcast address.
pub const LOCAL_BROADCAST: SocketAddrV4 = address(192, 168, 1, 255, 0xBAC0);
/// A device on its subnet.
pub const LOCAL_DEVICE: SocketAddrV4 = address(192, 168, 1, 20, 0xBAC0);
/// A peer reached by one-hop distribution, at its subnet's broadcast.
pub const ONE_HOP: SocketAddrV4 = address(10, 0, 0, 1, 0xBAC0);
/// A peer reached by two-hop distribution, itself.
pub const TWO_HOP: SocketAddrV4 = address(10, 1, 0, 1, 0xBAC0);
/// Foreign devices, in the order they register; the last finds no room.
pub const DEVICES: [SocketAddrV4; 5] = [
    address(172, 16, 0, 5, 47809),
    address(172, 16, 0, 6, 0xBAC0),
    address(172, 16, 0, 7, 0xBAC0),
    address(172, 16, 0, 8, 0xBAC0),
    address(172, 16, 0, 9, 0xBAC0),
];
/// Their times-to-live: the second lapses first, 60 s after registering.
pub const TTLS: [u16; 5] = [60, 30, 300, 60, 60];
/// The BDT: the own row is added.
pub const BDT: [BdtEntry; 2] = [
    BdtEntry::new(ONE_HOP, Ipv4Addr::new(255, 255, 255, 0)),
    BdtEntry::new(TWO_HOP, TWO_HOP_MASK),
];
/// Where the two-hop peer's forwarded broadcast started.
pub const ORIGIN: SocketAddrV4 = address(10, 1, 0, 77, 0xBAC0);
/// Foreign devices one decision goes to at most.
pub const FANOUT: usize = 3;

/// The BBMD's storage: its tables, one decision's destinations, and the
/// FDT rows a Read-FDT-Ack would carry.
pub struct Storage {
    tables: Bbmd,
    targets: [Target; TARGETS],
    rows: [FdtEntry; 4],
}

impl Storage {
    /// Empty storage.
    pub const fn new() -> Self {
        Self {
            // All zero bits, so the image's static goes in .bss: `run`
            // sets the own address.
            tables: Bbmd::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            targets: [Target::UNUSED; TARGETS],
            rows: [FdtEntry::new(address(0, 0, 0, 0, 0), 0, 0); 4],
        }
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}

/// One decision's destinations, in order, and whether the fan-out budget
/// cut them short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// The Forwarded-NPDU's originating address.
    pub origin: SocketAddrV4,
    /// The destinations, `None` past the last.
    pub targets: [Option<Target>; TARGETS],
    /// Whether the budget left foreign devices out.
    pub capped: bool,
}

impl Sent {
    fn of(sent: &Distribution<'_, '_>) -> Self {
        let mut targets = [None; TARGETS];
        for (slot, target) in targets.iter_mut().zip(sent.fanout.targets) {
            *slot = Some(*target);
        }
        Self {
            origin: sent.origin,
            targets,
            capped: sent.fanout.fanout_capped,
        }
    }
}

/// What [`run`] saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Rows the BDT holds once set.
    pub bdt_rows: Option<usize>,
    /// Registrations accepted.
    pub accepted: u8,
    /// Why the fifth device, and a time-to-live of 0, were refused.
    pub refusals: [Option<RegistrationRefusal>; 2],
    /// The local device's Original-Broadcast-NPDU.
    pub original: Option<Sent>,
    /// The two-hop peer's Forwarded-NPDU.
    pub forwarded: Option<Sent>,
    /// The same from a sender that is no peer.
    pub not_from_peer: Option<ForwardedRefusal>,
    /// The second device's Distribute-Broadcast-To-Network.
    pub distributed: Option<Sent>,
    /// The same from the local device, which isn't registered.
    pub unregistered: Option<BvlcResultCode>,
    /// Registrations purged 61 s on.
    pub purged: usize,
    /// The BBMD's own broadcast then.
    pub own: Option<Sent>,
    /// The FDT rows then, as a Read-FDT-Ack carries them, `None` past the
    /// last.
    pub fdt: [Option<FdtEntry>; 4],
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

impl Summary {
    pub(super) const fn new() -> Self {
        Self {
            bdt_rows: None,
            accepted: 0,
            refusals: [None; 2],
            original: None,
            forwarded: None,
            not_from_peer: None,
            distributed: None,
            unregistered: None,
            purged: 0,
            own: None,
            fdt: [None; 4],
            errors: 0,
        }
    }
}

/// Run the BBMD part in `storage`.
pub fn run(storage: &mut Storage) -> Summary {
    let Storage {
        tables,
        targets,
        rows,
    } = storage;
    let mut summary = Summary::new();
    *tables = Bbmd::new(OWN);
    summary.bdt_rows = tables
        .bdt
        .set(BdtEntries::from_entries(&BDT))
        .ok()
        .map(|()| tables.bdt.len());

    let t0 = Instant::from_secs(1_000);
    let limits = RegistrationLimits::default();
    for (device, ttl) in DEVICES.iter().zip(TTLS) {
        match tables
            .fdt
            .register(*device, ttl, t0, &limits, false, || true)
            .result
        {
            Ok(_) => summary.accepted += 1,
            Err(refusal) => summary.refusals[0] = Some(refusal),
        }
    }
    let zero = tables
        .fdt
        .register(DEVICES[0], 0, t0, &limits, false, || true);
    summary.refusals[1] = zero.result.err();

    let cx = Context {
        now: t0,
        local_broadcast: LOCAL_BROADCAST,
        fdt_fanout: FANOUT,
    };
    summary.original = Some(Sent::of(&tables.original_broadcast(
        &cx,
        LOCAL_DEVICE,
        &NPDU,
        targets,
    )));
    summary.forwarded = tables
        .forwarded_npdu(&cx, TWO_HOP, Arrival::Unicast, ORIGIN, &NPDU, targets)
        .ok()
        .map(|sent| Sent::of(&sent));
    summary.not_from_peer = tables
        .forwarded_npdu(&cx, LOCAL_DEVICE, Arrival::Unicast, ORIGIN, &NPDU, targets)
        .err();
    summary.distributed = tables
        .distribute_broadcast(&cx, DEVICES[1], &NPDU, targets)
        .ok()
        .map(|sent| Sent::of(&sent));
    summary.unregistered = tables
        .distribute_broadcast(&cx, LOCAL_DEVICE, &NPDU, targets)
        .err();

    let later = t0 + Duration::from_secs(61);
    summary.purged = tables.fdt.purge(later);
    let cx = Context { now: later, ..cx };
    summary.own = Some(Sent::of(&tables.own_broadcast(&cx, &NPDU, targets)));
    match tables.fdt.wire_entries(later, rows) {
        Ok(table) => {
            for (slot, row) in summary.fdt.iter_mut().zip(table) {
                *slot = Some(row);
            }
        }
        Err(_) => summary.errors += 1,
    }
    if summary.bdt_rows.is_none() || summary.forwarded.is_none() || summary.distributed.is_none() {
        summary.errors += 1;
    }
    summary
}
