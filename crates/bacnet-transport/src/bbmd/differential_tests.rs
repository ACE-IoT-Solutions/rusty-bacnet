//! The BBMD on `bacnet-bip-core` against the code it replaced (#1623).
//!
//! [`super::legacy`] is the old `BbmdState`, and the old receive path's
//! decisions are rebuilt from its methods below. Each case runs one
//! generated sequence of operations on both, at the same injected times,
//! and compares every result and, after each step, the tables and counters.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use bacnet_bip_codec::BvlcResultCode;
use bacnet_bip_core::bbmd::{Arrival, ForwardedRefusal};
use proptest::prelude::*;

use super::legacy;
use super::{BbmdState, BdtEntry, ForeignDevicePolicy, Forward};

/// One step of a run.
#[derive(Debug, Clone)]
enum Op {
    SetBdt(Vec<BdtEntry>),
    /// That many distinct valid peers, to reach the table's capacity.
    SetLargeBdt(usize),
    SetLocalAddress([u8; 4], u16),
    SetPolicy(Option<ForeignDevicePolicy>),
    Register([u8; 4], u16, u16),
    Delete([u8; 4], u16),
    Purge,
    OriginalBroadcast([u8; 4], u16),
    OwnBroadcast,
    Forwarded {
        sender: ([u8; 4], u16),
        origin: ([u8; 4], u16),
        broadcast: bool,
    },
    Distribute([u8; 4], u16),
    FdtTargets([u8; 4], u16),
}

fn ip() -> impl Strategy<Value = [u8; 4]> {
    (0u8..3, 1u8..6).prop_map(|(subnet, host)| [10, 0, subnet, host])
}

fn port() -> impl Strategy<Value = u16> {
    prop_oneof![Just(0xBAC0u16), Just(0xBAC1)]
}

fn endpoint() -> impl Strategy<Value = ([u8; 4], u16)> {
    (ip(), port())
}

fn row() -> impl Strategy<Value = BdtEntry> {
    let mask = prop_oneof![
        4 => Just([255, 255, 255, 255]),
        3 => Just([255, 255, 255, 0]),
        2 => Just([255, 255, 255, 248]),
        1 => Just([255, 255, 0, 0]),
    ];
    let valid = (endpoint(), mask).prop_map(|((ip, port), broadcast_mask)| BdtEntry {
        ip,
        port,
        broadcast_mask,
    });
    // Now and then a row the policy refuses.
    let invalid = prop_oneof![
        Just(BdtEntry {
            ip: [10, 0, 0, 9],
            port: 0,
            broadcast_mask: [255, 255, 255, 255]
        }),
        Just(BdtEntry {
            ip: [224, 0, 0, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255]
        }),
        Just(BdtEntry {
            ip: [10, 0, 0, 0],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 0]
        }),
        Just(BdtEntry {
            ip: [10, 0, 0, 9],
            port: 0xBAC0,
            broadcast_mask: [255, 0, 255, 0]
        }),
    ];
    prop_oneof![20 => valid, 1 => invalid]
}

fn policy() -> impl Strategy<Value = Option<ForeignDevicePolicy>> {
    let some = (
        (1u16..20, 30u16..400),
        1usize..4,
        prop::option::of(prop::collection::vec(ip(), 0..6)),
        prop_oneof![Just(0usize), Just(124), Just(127), Just(128), Just(200)],
        prop::collection::vec(ip(), 0..3),
        0usize..5,
        (1u32..6, 2u32..12, 1u64..10),
    )
        .prop_map(
            |(
                (min_ttl, max_ttl),
                per_source,
                allowed,
                reserved,
                reserved_sources,
                fanout,
                rate,
            )| {
                Some(ForeignDevicePolicy {
                    min_ttl,
                    max_ttl,
                    max_entries_per_source: per_source,
                    allowed_sources: allowed,
                    reserved_capacity: reserved,
                    reserved_sources,
                    max_fdt_fanout: fanout,
                    registration_rate_per_source: rate.0,
                    registration_rate_global: rate.1,
                    rate_window: Duration::from_secs(rate.2),
                })
            },
        );
    prop_oneof![1 => Just(None), 1 => Just(Some(ForeignDevicePolicy::default())), 4 => some]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => prop::collection::vec(row(), 0..8).prop_map(Op::SetBdt),
        1 => (120usize..=130).prop_map(Op::SetLargeBdt),
        1 => (ip(), port()).prop_map(|(ip, port)| Op::SetLocalAddress(ip, port)),
        2 => policy().prop_map(Op::SetPolicy),
        8 => (ip(), port(), prop_oneof![0u16..5, 5u16..90, Just(400), Just(u16::MAX)])
            .prop_map(|(ip, port, ttl)| Op::Register(ip, port, ttl)),
        1 => endpoint().prop_map(|(ip, port)| Op::Delete(ip, port)),
        1 => Just(Op::Purge),
        3 => endpoint().prop_map(|(ip, port)| Op::OriginalBroadcast(ip, port)),
        1 => Just(Op::OwnBroadcast),
        3 => (endpoint(), endpoint(), any::<bool>())
            .prop_map(|(sender, origin, broadcast)| Op::Forwarded { sender, origin, broadcast }),
        3 => endpoint().prop_map(|(ip, port)| Op::Distribute(ip, port)),
        1 => endpoint().prop_map(|(ip, port)| Op::FdtTargets(ip, port)),
    ]
}

/// Steps, each with the time that passes before it: whole seconds, plus a
/// fraction so expiry is tested off the second too.
fn run() -> impl Strategy<Value = Vec<(Duration, Op)>> {
    let step = prop_oneof![
        3 => Just(Duration::ZERO),
        3 => (0u64..40, 0u32..1_000_000_000).prop_map(|(s, n)| Duration::new(s, n)),
        1 => (60u64..200).prop_map(Duration::from_secs),
    ];
    prop::collection::vec((step, op()), 1..40)
}

/// An (ip, port) destination, as the old methods return them.
type Pair = ([u8; 4], u16);

fn address(endpoint: ([u8; 4], u16)) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(endpoint.0), endpoint.1)
}

fn targets(remote: &[SocketAddrV4]) -> Vec<([u8; 4], u16)> {
    remote.iter().map(|a| (a.ip().octets(), a.port())).collect()
}

const LOCAL_BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
const NPDU: &[u8] = &[0x01, 0x20];

/// What the old receive path decided for a Forwarded-NPDU: its own-echo
/// check, then the peer check, then the local rebroadcast rule and the
/// foreign devices.
fn legacy_forwarded(
    old: &mut legacy::BbmdState,
    sender: ([u8; 4], u16),
    origin: ([u8; 4], u16),
    broadcast: bool,
    now: Instant,
) -> Result<(bool, Vec<Pair>), ForwardedRefusal> {
    if sender == old.local_address() {
        return Err(ForwardedRefusal::OwnEcho);
    }
    if !old.is_bdt_peer(sender.0, sender.1) {
        return Err(ForwardedRefusal::NotFromPeer);
    }
    // The J.4.5 fix: this BBMD's own row, not the sender's, decides.
    let local = own_row_is_two_hop(old) && !broadcast;
    Ok((
        local,
        old.fdt_forwarding_targets_at(origin.0, origin.1, now),
    ))
}

/// Whether the old state's own BDT row is two-hop, or missing: how the
/// fixed rule (J.4.5) tells that peers send to this BBMD alone.
fn own_row_is_two_hop(old: &legacy::BbmdState) -> bool {
    let own = old.local_address();
    old.bdt()
        .iter()
        .find(|row| (row.ip, row.port) == own)
        .is_none_or(|row| row.broadcast_mask == [255; 4])
}

/// What the old receive path decided for a Distribute-Broadcast-To-Network.
fn legacy_distribute(
    old: &mut legacy::BbmdState,
    sender: ([u8; 4], u16),
    now: Instant,
) -> Result<Vec<([u8; 4], u16)>, BvlcResultCode> {
    if !old.is_registered_foreign_device_at(sender.0, sender.1, now) {
        return Err(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK);
    }
    Ok(old.forwarding_targets_at(sender.0, sender.1, now))
}

fn split(forward: Forward) -> (Option<SocketAddrV4>, Vec<([u8; 4], u16)>) {
    (forward.local_broadcast, targets(&forward.remote))
}

/// Everything observable about the tables and counters, compared after
/// every step.
fn assert_same_state(old: &mut legacy::BbmdState, new: &mut BbmdState, now: Instant) {
    assert_eq!(new.local_address(), old.local_address());
    assert_eq!(new.bdt(), old.bdt());
    assert_eq!(new.configured_bdt(), old.configured_bdt());
    assert_eq!(new.fdt_counters(), old.fdt_counters());
    let new_fdt: Vec<_> = new
        .tables
        .fdt
        .registrations()
        .iter()
        .map(|r| {
            let at = new.clock.to_std(r.registered_at);
            (r.address.ip().octets(), r.address.port(), r.ttl, at)
        })
        .collect();
    let old_fdt: Vec<_> = old
        .fdt
        .iter()
        .map(|e| (e.ip, e.port, e.ttl, e.registered_at))
        .collect();
    assert_eq!(new_fdt, old_fdt);
    let views: Vec<_> = new
        .fdt_entries_at(now)
        .iter()
        .map(|e| {
            let std::net::SocketAddr::V4(a) = e.address else {
                unreachable!()
            };
            (
                a.ip().octets(),
                a.port(),
                e.time_to_live,
                e.remaining_time_to_live,
            )
        })
        .collect();
    assert_eq!(views, old.fdt_entries_at(now));
    for (ip, port) in [([10, 0, 0, 1], 0xBAC0), ([10, 0, 1, 2], 0xBAC1)] {
        assert_eq!(new.is_bdt_peer(ip, port), old.is_bdt_peer(ip, port));
        let fixed = old.is_bdt_peer(ip, port)
            && (ip, port) != old.local_address()
            && own_row_is_two_hop(old);
        assert_eq!(new.forwarded_npdu_needs_local_broadcast(ip, port), fixed);
    }
}

/// Run `ops` on both from `own`, comparing as it goes.
fn compare(own: ([u8; 4], u16), ops: Vec<(Duration, Op)>) {
    let t0 = Instant::now();
    let mut old = legacy::BbmdState::new(own.0, own.1);
    let mut new = BbmdState::new(own.0, own.1);
    let mut now = t0;
    for (step, (elapsed, op)) in ops.into_iter().enumerate() {
        now += elapsed;
        let context = format!("step {step}: {op:?}");
        match op {
            Op::SetBdt(rows) => {
                let a = old.set_bdt(rows.clone()).map_err(|e| e.to_string());
                let b = new.set_bdt(rows).map_err(|e| e.to_string());
                assert_eq!(b, a, "{context}");
            }
            Op::SetLargeBdt(n) => {
                let rows: Vec<BdtEntry> = (0..n)
                    .map(|i| BdtEntry {
                        ip: [10, 9, (i / 200) as u8, (i % 200 + 1) as u8],
                        port: 0xBAC0,
                        broadcast_mask: [255, 255, 255, 255],
                    })
                    .collect();
                let a = old.set_bdt(rows.clone()).map_err(|e| e.to_string());
                let b = new.set_bdt(rows).map_err(|e| e.to_string());
                assert_eq!(b, a, "{context}");
            }
            Op::SetLocalAddress(ip, port) => {
                let a = old.set_local_address(ip, port).map_err(|e| e.to_string());
                let b = new.set_local_address(ip, port).map_err(|e| e.to_string());
                assert_eq!(b, a, "{context}");
            }
            Op::SetPolicy(policy) => {
                old.set_foreign_device_policy(policy.clone());
                new.set_foreign_device_policy(policy);
            }
            Op::Register(ip, port, ttl) => {
                let a = old.register_foreign_device_at(ip, port, ttl, now);
                let b = new.register_foreign_device_at(ip, port, ttl, now);
                assert_eq!(b, a, "{context}");
            }
            Op::Delete(ip, port) => {
                let a = old.delete_foreign_device(ip, port);
                let b = new.delete_foreign_device(ip, port);
                assert_eq!(b, a, "{context}");
            }
            Op::Purge => {
                assert_eq!(
                    new.purge_expired_at(now),
                    old.purge_expired_at(now),
                    "{context}"
                );
                let a = old.fdt_wire_entries_at(now);
                let b: Vec<_> = new
                    .fdt_wire_entries_at(now)
                    .iter()
                    .map(|e| {
                        (
                            e.address.ip().octets(),
                            e.address.port(),
                            e.ttl,
                            e.remaining,
                        )
                    })
                    .collect();
                assert_eq!(b, a, "{context}");
            }
            Op::OriginalBroadcast(ip, port) => {
                let a = old.forwarding_targets_at(ip, port, now);
                let forward = new.forward_original_broadcast(address((ip, port)), NPDU, now);
                assert_eq!(forward.origin, address((ip, port)));
                assert_eq!(split(forward), (None, a), "{context}");
            }
            Op::OwnBroadcast => {
                let own = old.local_address();
                let a = old.forwarding_targets_at(own.0, own.1, now);
                let forward = new.forward_own_broadcast(NPDU, now);
                assert_eq!(forward.origin, address(own));
                assert_eq!(split(forward), (None, a), "{context}");
            }
            Op::Forwarded {
                sender,
                origin,
                broadcast,
            } => {
                let a = legacy_forwarded(&mut old, sender, origin, broadcast, now);
                let arrival = if broadcast {
                    Arrival::Broadcast
                } else {
                    Arrival::Unicast
                };
                let b = new.forward_forwarded_npdu(
                    address(sender),
                    arrival,
                    address(origin),
                    LOCAL_BROADCAST,
                    now,
                );
                let b = b.map(|forward| {
                    assert_eq!(forward.origin, address(origin));
                    let (local, remote) = split(forward);
                    (local == Some(LOCAL_BROADCAST), remote)
                });
                assert_eq!(b, a, "{context}");
            }
            Op::Distribute(ip, port) => {
                let a = legacy_distribute(&mut old, (ip, port), now);
                let b = new
                    .forward_distribute_broadcast(address((ip, port)), NPDU, LOCAL_BROADCAST, now)
                    .map(|forward| {
                        assert_eq!(forward.origin, address((ip, port)));
                        let (local, remote) = split(forward);
                        assert_eq!(local, Some(LOCAL_BROADCAST));
                        remote
                    });
                assert_eq!(b, a, "{context}");
            }
            Op::FdtTargets(ip, port) => {
                let a = old.fdt_forwarding_targets_at(ip, port, now);
                let b = new.fdt_forwarding_targets_at(ip, port, now);
                assert_eq!(b, a, "{context}");
            }
        }
        assert_same_state(&mut old, &mut new, now);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn the_core_decides_as_the_old_bbmd_did(own in endpoint(), ops in run()) {
        compare(own, ops);
    }
}

/// A run that fills the FDT: an ordinary source stops short of the reserved
/// slots, a reserved one fills them, and the rate limit and quota apply.
#[test]
fn a_full_table_decides_the_same() {
    let policy = ForeignDevicePolicy {
        reserved_capacity: 2,
        reserved_sources: vec![[10, 7, 0, 1]],
        max_entries_per_source: 200,
        registration_rate_per_source: 1_000,
        registration_rate_global: 1_000,
        max_fdt_fanout: 200,
        ..ForeignDevicePolicy::default()
    };
    let mut ops = vec![(Duration::ZERO, Op::SetPolicy(Some(policy)))];
    for i in 0..130u16 {
        let ip = [10, 6, (i / 100) as u8, (i % 100 + 1) as u8];
        ops.push((Duration::from_millis(5), Op::Register(ip, 0xBAC0, 60)));
    }
    for port in 0..4u16 {
        ops.push((
            Duration::ZERO,
            Op::Register([10, 7, 0, 1], 0xBAC0 + port, 60),
        ));
    }
    ops.push((Duration::ZERO, Op::OriginalBroadcast([10, 0, 0, 1], 0xBAC0)));
    ops.push((Duration::from_secs(91), Op::Purge));
    compare(([10, 0, 0, 1], 0xBAC0), ops);
}

/// Registrations probed at exactly their time-to-live plus the grace
/// period, and a nanosecond later.
#[test]
fn expiry_at_the_boundary_decides_the_same() {
    let fd = ([10, 0, 1, 2], 0xBAC0);
    let mut ops = vec![
        (
            Duration::ZERO,
            Op::SetPolicy(Some(ForeignDevicePolicy::default())),
        ),
        (Duration::ZERO, Op::Register(fd.0, fd.1, 60)),
        (Duration::ZERO, Op::Register([10, 0, 1, 3], 0xBAC0, 1)),
    ];
    // The ttl-1 registration lapses past 31 s, the ttl-60 one past 90 s.
    for (step, op) in [
        (
            Duration::from_secs(31),
            Op::Distribute([10, 0, 1, 3], 0xBAC0),
        ),
        (
            Duration::from_nanos(1),
            Op::Distribute([10, 0, 1, 3], 0xBAC0),
        ),
        (
            Duration::from_secs(59) - Duration::from_nanos(1),
            Op::Distribute(fd.0, fd.1),
        ),
        (Duration::ZERO, Op::OriginalBroadcast([10, 0, 0, 1], 0xBAC1)),
        (Duration::ZERO, Op::Purge),
        (
            Duration::from_nanos(1),
            Op::OriginalBroadcast([10, 0, 0, 1], 0xBAC1),
        ),
        (Duration::ZERO, Op::Distribute(fd.0, fd.1)),
    ] {
        ops.push((step, op));
    }
    compare(([10, 0, 0, 1], 0xBAC0), ops);
}

/// More foreign devices than the default fan-out budget, which applies
/// once the policy that admitted them is gone.
#[test]
fn the_default_fanout_applies_without_a_policy() {
    let policy = ForeignDevicePolicy {
        max_entries_per_source: 200,
        registration_rate_per_source: 1_000,
        registration_rate_global: 1_000,
        ..ForeignDevicePolicy::default()
    };
    let mut ops = vec![(Duration::ZERO, Op::SetPolicy(Some(policy)))];
    for host in 1..=40u8 {
        ops.push((Duration::ZERO, Op::Register([10, 8, 0, host], 0xBAC0, 60)));
    }
    ops.push((Duration::ZERO, Op::SetPolicy(None)));
    ops.push((Duration::ZERO, Op::OriginalBroadcast([10, 0, 0, 1], 0xBAC1)));
    ops.push((Duration::ZERO, Op::FdtTargets([10, 8, 0, 1], 0xBAC0)));
    compare(([10, 0, 0, 1], 0xBAC0), ops);
}
