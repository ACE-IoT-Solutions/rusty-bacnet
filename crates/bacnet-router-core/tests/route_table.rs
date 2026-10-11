//! Bounded routing-table policy tests.

use bacnet_router_core::{
    LearnOutcome, LinkAddress, NetworkNumber, PortId, Reachability, RouteKind, RouteTable,
    RouteTableError,
};

fn net(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

#[test]
fn direct_and_learned_capacity_are_independent() {
    let mut table = RouteTable::<1, 1>::new();
    let hop = LinkAddress::try_from_slice(&[7]).unwrap();

    assert_eq!(
        table.learn(net(200), PortId(2), hop, 10),
        LearnOutcome::Inserted
    );
    assert_eq!(
        table.learn(net(201), PortId(2), hop, 10),
        LearnOutcome::TableFull
    );
    table.add_direct(net(100), PortId(1)).unwrap();
    assert_eq!(table.direct_len(), 1);
    assert_eq!(table.learned_len(), 1);
    assert_eq!(
        table.add_direct(net(101), PortId(3)),
        Err(RouteTableError::DirectTableFull)
    );
}

#[test]
fn learned_flood_cannot_replace_direct_route_and_full_table_can_refresh() {
    let mut table = RouteTable::<1, 1>::new();
    let first = LinkAddress::try_from_slice(&[7]).unwrap();
    let replacement = LinkAddress::try_from_slice(&[8]).unwrap();
    table.add_direct(net(100), PortId(1)).unwrap();

    assert_eq!(
        table.learn(net(100), PortId(2), first, 10),
        LearnOutcome::DirectConflict
    );
    assert_eq!(
        table.learn(net(200), PortId(2), first, 10),
        LearnOutcome::Inserted
    );
    assert_eq!(
        table.learn(net(200), PortId(3), replacement, 20),
        LearnOutcome::Updated
    );

    let direct = table.lookup(net(100)).unwrap();
    assert_eq!(direct.kind, RouteKind::Direct);
    assert_eq!(direct.port, PortId(1));
    let learned = table.lookup(net(200)).unwrap();
    assert_eq!(learned.port, PortId(3));
    assert_eq!(learned.next_hop, replacement);
    assert_eq!(learned.last_seen, Some(20));
}

#[test]
fn busy_deadline_is_effective_without_mutating_direct_routes() {
    let mut table = RouteTable::<1, 1>::new();
    let hop = LinkAddress::try_from_slice(&[7]).unwrap();
    table.add_direct(net(100), PortId(1)).unwrap();
    table.learn(net(200), PortId(2), hop, 10);

    assert!(!table.mark_busy(net(100), 50));
    assert!(table.mark_busy(net(200), 50));
    let learned = table.lookup(net(200)).unwrap();
    assert_eq!(
        learned.effective_reachability(49),
        Reachability::BusyUntil(50)
    );
    assert_eq!(learned.effective_reachability(50), Reachability::Reachable);
}

#[test]
fn network_control_mutation_requires_the_learned_route_owner() {
    let mut table = RouteTable::<1, 2>::new();
    let owner = LinkAddress::try_from_slice(&[7]).unwrap();
    let stranger = LinkAddress::try_from_slice(&[8]).unwrap();
    table.add_direct(net(100), PortId(1)).unwrap();
    table.learn(net(200), PortId(2), owner, 10);

    assert!(!table.mark_busy_from(net(100), PortId(1), owner, 50));
    assert!(!table.mark_busy_from(net(200), PortId(3), owner, 50));
    assert!(!table.mark_busy_from(net(200), PortId(2), stranger, 50));
    assert_eq!(
        table.lookup(net(200)).unwrap().reachability,
        Reachability::Reachable
    );

    assert!(table.mark_busy_from(net(200), PortId(2), owner, 50));
    assert!(!table.mark_available_from(net(200), PortId(2), stranger));
    assert_eq!(
        table.lookup(net(200)).unwrap().reachability,
        Reachability::BusyUntil(50)
    );
    assert!(table.mark_available_from(net(200), PortId(2), owner));
    assert!(table.mark_unreachable_from(net(200), PortId(2), owner));
    assert_eq!(
        table.lookup(net(200)).unwrap().reachability,
        Reachability::Unreachable
    );
    assert!(!table.remove_learned_from(net(200), PortId(2), stranger));
    assert!(table.remove_learned_from(net(200), PortId(2), owner));
    assert!(table.lookup(net(200)).is_none());
}

#[test]
fn snapshots_are_detached_and_clear_only_removes_learned_routes() {
    let mut table = RouteTable::<2, 2>::new();
    let hop = LinkAddress::try_from_slice(&[7]).unwrap();
    table.add_direct(net(100), PortId(1)).unwrap();
    table.learn(net(200), PortId(2), hop, 10);

    let entries: Vec<_> = table.entries().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(table.direct_entries().count(), 1);
    assert_eq!(table.learned_entries().count(), 1);
    assert_eq!(table.clear_learned(), 1);
    assert_eq!(table.direct_len(), 1);
    assert_eq!(table.learned_len(), 0);
    assert_eq!(entries[1].network, net(200));
}
