use std::mem::size_of;

use bacnet_router_core::{
    AdmissionOutcome, DiagnosticCounters, DiagnosticEvent, InvalidState, NetworkNumber,
    OperationalControl, OperationalControlResult, OperationalDiagnostics, OperationalError,
    PortCapabilities, PortId, PortKind, RecentFailure,
};

type Diagnostics = OperationalDiagnostics<2, 3, 4>;

fn capabilities(id: u8, max_npdu_len: usize) -> PortCapabilities {
    PortCapabilities {
        id: PortId(id),
        network: NetworkNumber::new(u16::from(id) + 1).expect("test network is routable"),
        kind: PortKind::MstpStandard,
        max_npdu_len,
    }
}

#[test]
fn registration_is_bounded_and_snapshot_is_immutable() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    assert_eq!(diagnostics.registered_ports(), 1);
    let snapshot = diagnostics.snapshot();
    assert_eq!(snapshot.registered_ports(), 1);
    let (port, counters) = snapshot.port(PortId(1)).unwrap();
    assert_eq!(port.capabilities, capabilities(1, 501));
    assert!(port.enabled);
    assert_eq!(port.last_queue_outcome, None);
    assert_eq!(port.recent_failure, None);
    assert_eq!(counters, DiagnosticCounters::default());
    assert_eq!(snapshot.route_occupancy.used, 0);
    assert_eq!(snapshot.route_occupancy.capacity, 3);
    assert_eq!(snapshot.cache_occupancy.capacity, 4);
}

#[test]
fn every_event_updates_global_and_registered_port_counters() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    let events = [
        DiagnosticEvent::Accepted,
        DiagnosticEvent::Full,
        DiagnosticEvent::Oversize,
        DiagnosticEvent::Expired,
        DiagnosticEvent::Malformed,
        DiagnosticEvent::Duplicate,
        DiagnosticEvent::PolicyDrop,
        DiagnosticEvent::Rejection,
        DiagnosticEvent::TransportFailure,
    ];
    for event in events {
        diagnostics.record_event(Some(PortId(1)), event).unwrap();
    }
    assert_eq!(
        diagnostics.global_counters(),
        DiagnosticCounters {
            accepted: 1,
            full: 1,
            oversize: 1,
            expired: 1,
            malformed: 1,
            duplicate: 1,
            policy_drop: 1,
            rejection: 1,
            transport_failure: 1,
        }
    );
    let (port, counters) = diagnostics.port(PortId(1)).unwrap();
    assert_eq!(counters, diagnostics.global_counters());
    assert_eq!(port.recent_failure, Some(RecentFailure::TransportFailure));
}

#[test]
fn counters_saturate_exactly_at_u64_maximum() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    diagnostics
        .record_event_count(None, DiagnosticEvent::Accepted, u64::MAX)
        .unwrap();
    diagnostics
        .record_event(None, DiagnosticEvent::Accepted)
        .unwrap();
    diagnostics
        .record_event_count(Some(PortId(1)), DiagnosticEvent::Full, u64::MAX)
        .unwrap();
    diagnostics
        .record_event_count(Some(PortId(1)), DiagnosticEvent::Full, 9)
        .unwrap();
    assert_eq!(diagnostics.global_counters().accepted, u64::MAX);
    assert_eq!(diagnostics.port(PortId(1)).unwrap().1.full, u64::MAX);
}

#[test]
fn queue_outcomes_map_to_counters_and_retain_last_queue_result() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    for outcome in [
        AdmissionOutcome::AcceptedOwned,
        AdmissionOutcome::Full,
        AdmissionOutcome::Oversize,
        AdmissionOutcome::Expired,
        AdmissionOutcome::Invalid,
        AdmissionOutcome::PortDown,
    ] {
        diagnostics
            .observe_queue_outcome(PortId(1), outcome)
            .unwrap();
    }
    let (port, counters) = diagnostics.port(PortId(1)).unwrap();
    assert_eq!(port.last_queue_outcome, Some(AdmissionOutcome::PortDown));
    assert_eq!(port.recent_failure, Some(RecentFailure::TransportFailure));
    assert_eq!(counters.accepted, 1);
    assert_eq!(counters.full, 1);
    assert_eq!(counters.oversize, 1);
    assert_eq!(counters.expired, 1);
    assert_eq!(counters.malformed, 1);
    assert_eq!(counters.transport_failure, 1);
}

#[test]
fn controls_are_synchronous_requests_with_explicit_results() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    assert_eq!(
        diagnostics
            .apply(OperationalControl::SetPortEnabled {
                port: PortId(1),
                enabled: false,
            })
            .unwrap(),
        OperationalControlResult::PortState {
            port: PortId(1),
            enabled: false,
            changed: true,
        }
    );
    assert_eq!(
        diagnostics
            .apply(OperationalControl::SetPortEnabled {
                port: PortId(1),
                enabled: false,
            })
            .unwrap(),
        OperationalControlResult::PortState {
            port: PortId(1),
            enabled: false,
            changed: false,
        }
    );
    assert_eq!(
        diagnostics
            .apply(OperationalControl::ClearLearnedRoutes)
            .unwrap(),
        OperationalControlResult::LearnedRoutesClearRequested
    );
    assert_eq!(
        diagnostics
            .apply(OperationalControl::RequestLearnedRouteRefresh)
            .unwrap(),
        OperationalControlResult::LearnedRoutesRefreshRequested
    );
    assert_eq!(
        diagnostics
            .apply(OperationalControl::AnnounceDiscovery)
            .unwrap(),
        OperationalControlResult::DiscoveryAnnouncementRequested
    );
}

#[test]
fn reset_preserves_control_state_and_occupancy_but_clears_diagnostics() {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    diagnostics.set_route_occupancy(2).unwrap();
    diagnostics.set_cache_occupancy(3).unwrap();
    diagnostics
        .apply(OperationalControl::SetPortEnabled {
            port: PortId(1),
            enabled: false,
        })
        .unwrap();
    diagnostics
        .observe_queue_outcome(PortId(1), AdmissionOutcome::Full)
        .unwrap();
    assert_eq!(
        diagnostics
            .apply(OperationalControl::ResetDiagnostics)
            .unwrap(),
        OperationalControlResult::DiagnosticsReset
    );
    let snapshot = diagnostics.snapshot();
    let (port, counters) = snapshot.port(PortId(1)).unwrap();
    assert!(!port.enabled);
    assert_eq!(port.last_queue_outcome, None);
    assert_eq!(port.recent_failure, None);
    assert_eq!(counters, DiagnosticCounters::default());
    assert_eq!(snapshot.global, DiagnosticCounters::default());
    assert_eq!(snapshot.route_occupancy.used, 2);
    assert_eq!(snapshot.cache_occupancy.used, 3);
}

#[test]
fn full_unknown_and_invalid_state_results_are_deterministic() {
    let mut diagnostics = Diagnostics::new();
    assert_eq!(
        diagnostics.register_port(capabilities(1, 0)),
        Err(OperationalError::InvalidState(
            InvalidState::ZeroPortNpduCapacity(PortId(1),)
        ))
    );
    diagnostics.register_port(capabilities(1, 501)).unwrap();
    assert_eq!(
        diagnostics.register_port(capabilities(1, 501)),
        Err(OperationalError::InvalidState(
            InvalidState::PortAlreadyRegistered(PortId(1),)
        ))
    );
    diagnostics.register_port(capabilities(2, 501)).unwrap();
    assert_eq!(
        diagnostics.register_port(capabilities(3, 501)),
        Err(OperationalError::PortTableFull { capacity: 2 })
    );
    assert_eq!(
        diagnostics.is_port_enabled(PortId(99)),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(
        diagnostics.record_event(Some(PortId(99)), DiagnosticEvent::Malformed),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(
        diagnostics.set_route_occupancy(4),
        Err(OperationalError::InvalidState(
            InvalidState::RouteOccupancyExceedsCapacity {
                used: 4,
                capacity: 3,
            },
        ))
    );
    assert_eq!(
        diagnostics.set_cache_occupancy(5),
        Err(OperationalError::InvalidState(
            InvalidState::CacheOccupancyExceedsCapacity {
                used: 5,
                capacity: 4,
            },
        ))
    );
}

#[test]
fn representative_memory_budgets_remain_bounded() {
    assert_eq!(size_of::<DiagnosticCounters>(), 9 * size_of::<u64>());
    assert!(size_of::<OperationalDiagnostics<4, 32, 64>>() <= 2_048);
    assert!(size_of::<OperationalDiagnostics<8, 64, 128>>() <= 4_096);
}
