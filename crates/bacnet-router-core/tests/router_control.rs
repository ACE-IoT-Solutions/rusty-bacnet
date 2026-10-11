//! Router discovery and routing-table control state tests.

use bacnet_router_core::{
    AdmissionOutcome, ControlDelivery, LinkAddress, NetworkNumber, PortId, PortIdSemantics,
    Reachability, RouterControl, RouterControlAction, RouterControlError, RouterControlRequest,
};

const WHO_IS: u8 = 0x00;
const I_AM: u8 = 0x01;
const BUSY: u8 = 0x04;
const AVAILABLE: u8 = 0x05;
const INIT: u8 = 0x06;
const INIT_ACK: u8 = 0x07;

fn net(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

fn peer(value: u8) -> LinkAddress {
    LinkAddress::try_from_slice(&[value]).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn handle<'a, const D: usize, const L: usize>(
    state: &'a mut RouterControl<D, L>,
    ingress: PortId,
    source: LinkAddress,
    message_type: u8,
    payload: &'a [u8],
    now: u64,
    networks: &'a mut [NetworkNumber],
    snapshot: &'a mut [bacnet_router_core::RouteEntry],
) -> Result<bacnet_router_core::RouterControlOutcome<'a>, RouterControlError> {
    state.handle(RouterControlRequest {
        ingress,
        peer: source,
        message_type,
        payload,
        now,
        network_out: networks,
        snapshot_out: snapshot,
    })
}

#[test]
fn who_is_answers_local_routes_and_forwards_unknown_queries() {
    let mut state = RouterControl::<2, 2>::new();
    state.add_direct(net(100), PortId(1)).unwrap();
    let mut networks = [net(999); 4];
    let mut snapshot = [state.routes().entries().next().unwrap(); 4];

    let response = handle(
        &mut state,
        PortId(2),
        peer(7),
        WHO_IS,
        &[0, 100],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    match response.action {
        RouterControlAction::RespondIAm { port, networks } => {
            assert_eq!(port, PortId(2));
            assert_eq!(networks.as_slice(), &[net(100)]);
        }
        other => panic!("unexpected action: {other:?}"),
    }

    let forwarded = handle(
        &mut state,
        PortId(1),
        peer(7),
        WHO_IS,
        &[0, 200],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(
        forwarded.action,
        RouterControlAction::ForwardWhoIs {
            exclude: PortId(1),
            network: Some(net(200)),
        }
    );

    let all = handle(
        &mut state,
        PortId(2),
        peer(7),
        WHO_IS,
        &[],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    match all.action {
        RouterControlAction::RespondIAm { networks, .. } => {
            assert_eq!(networks.as_slice(), &[net(100)]);
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn i_am_learns_with_local_owner_and_controlled_rebroadcast() {
    let mut state = RouterControl::<1, 2>::new();
    state.add_direct(net(100), PortId(1)).unwrap();
    let mut networks = [net(999); 2];
    let mut snapshot = [state.routes().entries().next().unwrap(); 3];

    let outcome = handle(
        &mut state,
        PortId(2),
        peer(42),
        I_AM,
        &[0, 200, 0, 201],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(outcome.inserted, 2);
    match outcome.action {
        RouterControlAction::RebroadcastIAm { exclude, networks } => {
            assert_eq!(exclude, PortId(2));
            assert_eq!(networks.len(), 2);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    let route = state.routes().lookup(net(200)).unwrap();
    assert_eq!(route.port, PortId(2));
    assert_eq!(route.next_hop, peer(42));

    let direct_conflict = handle(
        &mut state,
        PortId(2),
        peer(42),
        I_AM,
        &[0, 100],
        11,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(direct_conflict.direct_conflicts, 1);
    assert_eq!(direct_conflict.action, RouterControlAction::None);
    assert_eq!(state.routes().lookup(net(100)).unwrap().port, PortId(1));
}

#[test]
fn malformed_initialize_table_is_transactional() {
    // count=2; both entries have empty port-info. Every strict prefix is
    // malformed for both the request and ACK forms because the advertised
    // count cannot be satisfied.
    let complete = [2, 0, 200, 4, 0, 0, 201, 5, 0];
    for message_type in [INIT, INIT_ACK] {
        for length in 0..complete.len() {
            assert_malformed_initialize_does_not_mutate(message_type, &complete[..length]);
        }
        for malformed in [
            &[1, 0, 0, 4, 0][..],
            &[1, 0xff, 0xff, 4, 0][..],
            &[1, 0, 200, 4, 0, 0][..],
        ] {
            assert_malformed_initialize_does_not_mutate(message_type, malformed);
        }
    }
}

fn assert_malformed_initialize_does_not_mutate(message_type: u8, payload: &[u8]) {
    let mut state = RouterControl::<1, 2>::new();
    state.add_direct(net(100), PortId(1)).unwrap();
    let mut networks = [net(999); 2];
    let mut snapshot = [state.routes().entries().next().unwrap(); 3];

    let error = handle(
        &mut state,
        PortId(2),
        peer(9),
        message_type,
        payload,
        5,
        &mut networks,
        &mut snapshot,
    )
    .unwrap_err();

    assert!(matches!(error, RouterControlError::Codec(_)));
    assert_eq!(state.routes().direct_len(), 1);
    assert_eq!(state.routes().learned_len(), 0);
    assert_eq!(state.routes().lookup(net(100)).unwrap().port, PortId(1));
    assert!(state.routes().lookup(net(200)).is_none());
}

#[test]
fn full_table_refreshes_existing_without_eviction_or_direct_overwrite() {
    let mut state = RouterControl::<1, 1>::new();
    state.add_direct(net(100), PortId(1)).unwrap();
    let mut networks = [net(999); 2];
    let mut snapshot = [state.routes().entries().next().unwrap(); 2];
    handle(
        &mut state,
        PortId(2),
        peer(8),
        I_AM,
        &[0, 200],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    // A table containing only an existing learned route and a direct conflict
    // is accepted, even though the learned partition is full.
    let refresh = [2, 0, 100, 99, 0, 0, 200, 77, 0];
    let outcome = handle(
        &mut state,
        PortId(2),
        peer(8),
        INIT,
        &refresh,
        20,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(outcome.updated, 1);
    assert_eq!(outcome.direct_conflicts, 1);
    match outcome.action {
        RouterControlAction::SendRoutingTableAck { table, .. } => assert_eq!(table.len(), 0),
        other => panic!("unexpected action: {other:?}"),
    }
    assert_eq!(state.routes().lookup(net(100)).unwrap().port, PortId(1));
    assert_eq!(state.routes().lookup(net(200)).unwrap().last_seen, Some(20));

    // A genuinely new entry is rejected before any part of the transaction is
    // applied, so the existing owner remains unchanged.
    let full = [2, 0, 200, 77, 0, 0, 201, 77, 0];
    let error = handle(
        &mut state,
        PortId(3),
        peer(9),
        INIT_ACK,
        &full,
        21,
        &mut networks,
        &mut snapshot,
    )
    .unwrap_err();
    assert_eq!(
        error,
        RouterControlError::LearnedTableFull {
            needed: 1,
            available: 0
        }
    );
    assert_eq!(state.routes().lookup(net(200)).unwrap().port, PortId(2));
    assert!(state.routes().lookup(net(201)).is_none());
}

#[test]
fn busy_available_and_aging_use_exact_caller_time_and_owner() {
    let mut state = RouterControl::<0, 1>::new()
        .with_busy_duration(30)
        .with_learned_age(100);
    let mut networks = [net(999); 2];
    let mut snapshot: [bacnet_router_core::RouteEntry; 0] = [];
    handle(
        &mut state,
        PortId(2),
        peer(8),
        I_AM,
        &[0, 200],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();

    let wrong_owner = handle(
        &mut state,
        PortId(3),
        peer(9),
        BUSY,
        &[0, 200],
        20,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(wrong_owner.action, RouterControlAction::None);
    assert_eq!(
        state.routes().lookup(net(200)).unwrap().reachability,
        Reachability::Reachable
    );

    handle(
        &mut state,
        PortId(2),
        peer(8),
        BUSY,
        &[0, 200],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(
        state
            .routes()
            .lookup(net(200))
            .unwrap()
            .effective_reachability(39),
        Reachability::BusyUntil(40)
    );
    assert_eq!(
        state
            .routes()
            .lookup(net(200))
            .unwrap()
            .effective_reachability(40),
        Reachability::Reachable
    );
    let available = handle(
        &mut state,
        PortId(2),
        peer(8),
        AVAILABLE,
        &[0, 200],
        40,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert!(matches!(
        available.action,
        RouterControlAction::RebroadcastAvailable { .. }
    ));

    assert_eq!(state.age(109), 0);
    assert_eq!(state.age(110), 1);
    assert!(state.routes().lookup(net(200)).is_none());
}

#[test]
fn empty_busy_and_available_lists_apply_to_every_route_owned_by_peer() {
    let mut state = RouterControl::<0, 3>::new().with_busy_duration(30);
    for (network, owner) in [(200, peer(8)), (201, peer(8)), (202, peer(9))] {
        state.routes_mut().learn(net(network), PortId(2), owner, 1);
    }
    let mut networks = [net(999); 2];
    let mut snapshot: [bacnet_router_core::RouteEntry; 0] = [];

    let busy = handle(
        &mut state,
        PortId(2),
        peer(8),
        BUSY,
        &[],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert!(matches!(
        busy.action,
        RouterControlAction::RebroadcastBusy { .. }
    ));
    assert_eq!(
        state.routes().lookup(net(200)).unwrap().reachability,
        Reachability::BusyUntil(40)
    );
    assert_eq!(
        state.routes().lookup(net(201)).unwrap().reachability,
        Reachability::BusyUntil(40)
    );
    assert_eq!(
        state.routes().lookup(net(202)).unwrap().reachability,
        Reachability::Reachable
    );

    let available = handle(
        &mut state,
        PortId(2),
        peer(8),
        AVAILABLE,
        &[],
        20,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert!(matches!(
        available.action,
        RouterControlAction::RebroadcastAvailable { .. }
    ));
    assert_eq!(
        state.routes().lookup(net(200)).unwrap().reachability,
        Reachability::Reachable
    );
    assert_eq!(
        state.routes().lookup(net(201)).unwrap().reachability,
        Reachability::Reachable
    );
}

#[test]
fn query_ack_is_detached_and_unsupported_is_explicit() {
    let mut state = RouterControl::<1, 1>::new();
    state.add_direct(net(100), PortId(1)).unwrap();
    state.routes_mut().learn(net(200), PortId(2), peer(7), 0);
    let mut networks = [net(999); 1];
    let mut snapshot = [state.routes().entries().next().unwrap(); 2];
    let result = handle(
        &mut state,
        PortId(2),
        peer(7),
        INIT,
        &[0],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    let table = match result.action {
        RouterControlAction::SendRoutingTableAck { table, .. } => table,
        other => panic!("unexpected action: {other:?}"),
    };
    assert_eq!(
        table.len(),
        1,
        "learned routes are not part of the ACK table"
    );
    assert_eq!(table.as_slice()[0].network, net(100));
    let mut encoded = [0u8; 8];
    assert_eq!(table.encode(&mut encoded), Ok(5));
    assert_eq!(&encoded[..5], &[1, 0, 100, 1, 0]);

    let unsupported = handle(
        &mut state,
        PortId(2),
        peer(7),
        0x7f,
        &[],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(
        unsupported.action,
        RouterControlAction::Unsupported { message_type: 0x7f }
    );
    assert_eq!(state.port_id_semantics(), PortIdSemantics::RemoteOpaque);
    assert_eq!(
        ControlDelivery::from_admission(AdmissionOutcome::Full),
        ControlDelivery::Rejected(AdmissionOutcome::Full)
    );
}

#[test]
fn empty_peer_is_rejected_before_learning_or_busy_mutation() {
    let mut state = RouterControl::<0, 2>::new();
    state.routes_mut().learn(net(300), PortId(1), peer(1), 0);
    let mut networks = [net(999); 1];
    let mut snapshot = [state.routes().lookup(net(300)).unwrap(); 1];
    let error = handle(
        &mut state,
        PortId(2),
        LinkAddress::default(),
        I_AM,
        &[0, 200],
        1,
        &mut networks,
        &mut snapshot,
    )
    .unwrap_err();
    assert_eq!(error, RouterControlError::MissingPeerAddress);
    assert_eq!(state.routes().learned_len(), 1);
}

#[test]
fn mixed_owner_busy_and_available_rebroadcast_only_authorized_networks() {
    let mut state = RouterControl::<0, 3>::new().with_busy_duration(30);
    state.routes_mut().learn(net(200), PortId(2), peer(8), 1);
    state.routes_mut().learn(net(201), PortId(2), peer(9), 1);
    state.routes_mut().learn(net(202), PortId(2), peer(8), 1);
    let mut networks = [net(999); 2];
    let mut snapshot: [bacnet_router_core::RouteEntry; 0] = [];

    let busy = handle(
        &mut state,
        PortId(2),
        peer(8),
        BUSY,
        &[0, 200, 0, 201, 0, 202],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    match busy.action {
        RouterControlAction::RebroadcastBusy { networks, .. } => {
            assert_eq!(networks.as_slice(), &[net(200), net(202)]);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert_eq!(
        state.routes().lookup(net(201)).unwrap().reachability,
        Reachability::Reachable
    );

    let available = handle(
        &mut state,
        PortId(2),
        peer(8),
        AVAILABLE,
        &[0, 200, 0, 201, 0, 202],
        20,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    match available.action {
        RouterControlAction::RebroadcastAvailable { networks, .. } => {
            assert_eq!(networks.as_slice(), &[net(200), net(202)]);
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
fn i_am_reports_capacity_loss_and_rebroadcasts_only_retained_networks() {
    let mut state = RouterControl::<0, 1>::new();
    let mut networks = [net(999); 2];
    let mut snapshot: [bacnet_router_core::RouteEntry; 0] = [];
    let outcome = handle(
        &mut state,
        PortId(2),
        peer(8),
        I_AM,
        &[0, 200, 0, 201],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(outcome.inserted, 1);
    assert_eq!(outcome.capacity_lost, 1);
    match outcome.action {
        RouterControlAction::RebroadcastIAm { networks, .. } => {
            assert_eq!(networks.as_slice(), &[net(200)]);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert!(state.routes().lookup(net(201)).is_none());
}

#[test]
fn i_am_with_zero_capacity_reports_loss_without_advertising() {
    let mut state = RouterControl::<0, 0>::new();
    let mut networks: [NetworkNumber; 0] = [];
    let mut snapshot: [bacnet_router_core::RouteEntry; 0] = [];
    let outcome = handle(
        &mut state,
        PortId(2),
        peer(8),
        I_AM,
        &[0, 200],
        10,
        &mut networks,
        &mut snapshot,
    )
    .unwrap();
    assert_eq!(outcome.capacity_lost, 1);
    assert_eq!(outcome.action, RouterControlAction::None);
}

#[test]
fn age_at_zero_uses_inclusive_expiry_boundary() {
    let mut state = RouterControl::<0, 1>::new().with_learned_age(100);
    state.routes_mut().learn(net(200), PortId(2), peer(8), 0);
    assert_eq!(state.age(99), 0);
    assert_eq!(state.routes().learned_len(), 1);
    assert_eq!(state.age(100), 1);
    assert_eq!(state.routes().learned_len(), 0);
}
