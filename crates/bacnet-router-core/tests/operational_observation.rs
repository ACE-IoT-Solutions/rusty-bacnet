//! Exactly-once mappings from router boundary results to operational counters.

use bacnet_router_core::{
    AdmissionOutcome, BroadcastError, BroadcastFanoutOutcome, BroadcastFanoutResult,
    ControlDelivery, DiagnosticCounters, ForwardAttempt, ForwardError, ForwardOutcome, LinkAddress,
    NetworkNumber, OperationalDiagnostics, OperationalError, PortCapabilities, PortId, PortKind,
    RouterControlAction, RouterControlError, RouterControlOutcome,
};

type Diagnostics = OperationalDiagnostics<2, 2, 2>;

fn port(id: u8) -> PortCapabilities {
    PortCapabilities {
        id: PortId(id),
        network: NetworkNumber::new(u16::from(id) + 1).unwrap(),
        kind: PortKind::MstpStandard,
        max_npdu_len: 501,
    }
}

fn registered() -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(port(1)).unwrap();
    diagnostics.register_port(port(2)).unwrap();
    diagnostics
}

#[test]
fn forwarding_attempt_counts_result_once_and_ignores_rejection_metadata() {
    let mut diagnostics = registered();
    let source = LinkAddress::try_from_slice(&[7]).unwrap();
    diagnostics
        .observe_forwarding_attempt(
            PortId(1),
            ForwardAttempt {
                result: Err(ForwardError::NoRoute),
                rejection: Some(bacnet_router_core::RejectMessage {
                    reason: bacnet_router_core::ForwardRejectReason::NotDirectlyConnected,
                    network: 100,
                    source: bacnet_router_core::RefusalSource::Ingress {
                        network: NetworkNumber::new(2).unwrap(),
                        address: source,
                    },
                }),
            },
        )
        .unwrap();
    let counters = diagnostics.port(PortId(1)).unwrap().1;
    assert_eq!(counters.rejection, 1);
    assert_eq!(counters.accepted, 0);
    assert_eq!(counters.malformed, 0);

    diagnostics
        .observe_forwarding_result(
            PortId(1),
            Ok(ForwardOutcome::Forwarded { final_npdu_len: 12 }),
        )
        .unwrap();
    let counters = diagnostics.port(PortId(1)).unwrap().1;
    assert_eq!(counters.accepted, 1);
    assert_eq!(counters.rejection, 1);
}

#[test]
fn broadcast_error_is_ingress_once_and_selection_only_targets_are_free() {
    let mut diagnostics = registered();
    diagnostics
        .observe_broadcast_error(
            PortId(1),
            BroadcastError::ResultsTooSmall {
                needed: 2,
                available: 1,
            },
        )
        .unwrap();
    assert_eq!(diagnostics.global_counters().full, 1);

    for outcome in [
        BroadcastFanoutOutcome::IngressExcluded,
        BroadcastFanoutOutcome::NotDestinationNetwork,
    ] {
        diagnostics
            .observe_broadcast_target(BroadcastFanoutResult {
                port: PortId(2),
                outcome,
            })
            .unwrap();
    }
    assert_eq!(diagnostics.global_counters().full, 1);

    diagnostics
        .observe_broadcast_target(BroadcastFanoutResult {
            port: PortId(2),
            outcome: BroadcastFanoutOutcome::Full,
        })
        .unwrap();
    assert_eq!(diagnostics.global_counters().full, 2);
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.full, 1);
}

#[test]
fn control_processing_and_response_admission_are_distinct() {
    let mut diagnostics = registered();
    let outcome = RouterControlOutcome {
        action: RouterControlAction::None,
        inserted: 1,
        updated: 0,
        direct_conflicts: 0,
        capacity_lost: 2,
    };
    diagnostics
        .observe_control_result(PortId(1), Ok(outcome))
        .unwrap();
    let counters = diagnostics.port(PortId(1)).unwrap().1;
    assert_eq!(counters.accepted, 1);
    assert_eq!(counters.full, 2);

    diagnostics
        .observe_control_delivery(PortId(1), ControlDelivery::Rejected(AdmissionOutcome::Full))
        .unwrap();
    let counters = diagnostics.port(PortId(1)).unwrap().1;
    assert_eq!(counters.accepted, 1);
    assert_eq!(counters.full, 3);

    diagnostics
        .observe_control_delivery(PortId(1), ControlDelivery::NotNeeded)
        .unwrap();
    assert_eq!(diagnostics.port(PortId(1)).unwrap().1, counters);
}

#[test]
fn unknown_ports_do_not_mutate_even_for_noop_observations() {
    let mut diagnostics = registered();
    let before = diagnostics.snapshot().global;
    assert_eq!(
        diagnostics.observe_forwarding_result(
            PortId(99),
            Ok(ForwardOutcome::Forwarded { final_npdu_len: 1 }),
        ),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(diagnostics.snapshot().global, before);
    assert_eq!(
        diagnostics.observe_broadcast_error(PortId(99), BroadcastError::EmptyNpdu),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(diagnostics.snapshot().global, before);
    assert_eq!(
        diagnostics.observe_broadcast_target(BroadcastFanoutResult {
            port: PortId(99),
            outcome: BroadcastFanoutOutcome::IngressExcluded,
        }),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(diagnostics.snapshot().global, before);
    assert_eq!(
        diagnostics.observe_control_delivery(PortId(99), ControlDelivery::NotNeeded),
        Err(OperationalError::UnknownPort(PortId(99)))
    );
    assert_eq!(diagnostics.snapshot().global, before);
}

#[test]
fn reset_clears_observer_counters_without_removing_registration() {
    let mut diagnostics = registered();
    diagnostics
        .observe_forwarding_result(PortId(1), Ok(ForwardOutcome::QueueFull))
        .unwrap();
    diagnostics
        .observe_broadcast_target(BroadcastFanoutResult {
            port: PortId(2),
            outcome: BroadcastFanoutOutcome::Duplicate,
        })
        .unwrap();
    diagnostics.reset_diagnostics();
    assert_eq!(diagnostics.global_counters(), DiagnosticCounters::ZERO);
    assert_eq!(diagnostics.registered_ports(), 2);
    assert_eq!(
        diagnostics.port(PortId(1)).unwrap().1,
        DiagnosticCounters::ZERO
    );
    assert_eq!(
        diagnostics.port(PortId(2)).unwrap().1,
        DiagnosticCounters::ZERO
    );
}

#[test]
fn control_processing_errors_are_one_bounded_event() {
    let mut diagnostics = registered();
    diagnostics
        .observe_control_result(
            PortId(1),
            Err(RouterControlError::ResponseBufferTooSmall {
                needed: 2,
                capacity: 1,
            }),
        )
        .unwrap();
    assert_eq!(diagnostics.port(PortId(1)).unwrap().1.full, 1);
    assert_eq!(diagnostics.port(PortId(1)).unwrap().1.accepted, 0);
}
