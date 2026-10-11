//! Source-aware Reject-Message-To-Network action tests.

use bacnet_router_core::{
    AdmissionOutcome, DirectForwardRequest, DiscoveryAction, Egress, EgressRequest, ForwardError,
    ForwardOutcome, LinkAddress, NetworkNumber, PortCapabilities, PortId, PortKind, RouteDiscovery,
    RouterCore,
};

#[derive(Default)]
struct TestEgress {
    outcome: Option<AdmissionOutcome>,
    calls: usize,
}

impl Egress for TestEgress {
    fn try_admit(&mut self, _request: EgressRequest<'_>) -> AdmissionOutcome {
        self.calls += 1;
        self.outcome.unwrap_or(AdmissionOutcome::AcceptedOwned)
    }
}

fn routed_npdu(total_len: usize) -> Vec<u8> {
    assert!(total_len >= 7);
    let mut npdu = vec![0x5a; total_len];
    npdu[..7].copy_from_slice(&[0x01, 0x23, 0x00, 0xc8, 0x01, 0x05, 0xff]);
    npdu
}

fn routed_npdu_with_source() -> Vec<u8> {
    let mut npdu = vec![
        0x01, 0x2b, 0x00, 0xc8, 0x01, 0x05, 0x00, 0x4d, 0x02, 0x09, 0x0a, 0xff, 0x55, 0xaa,
    ];
    npdu.extend_from_slice(&[0x12, 0x34]);
    npdu
}

fn configured_core() -> RouterCore<2, 2> {
    let mut core = RouterCore::new();
    core.routes_mut()
        .add_direct(NetworkNumber::new(200).unwrap(), PortId(2))
        .unwrap();
    core
}

fn capabilities() -> PortCapabilities {
    PortCapabilities {
        id: PortId(2),
        network: NetworkNumber::new(200).unwrap(),
        kind: PortKind::MstpStandard,
        max_npdu_len: 501,
    }
}

#[test]
fn no_route_refusal_prefers_embedded_source_and_has_exact_wire() {
    let mut core = RouterCore::<2, 2>::new();
    let ingress_source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let request = DirectForwardRequest {
        ingress_network: NetworkNumber::new(100).unwrap(),
        logical_source: ingress_source,
        npdu: &routed_npdu_with_source(),
        egress_capabilities: capabilities(),
        now: 0,
    };
    let mut scratch = [0u8; 512];
    let mut egress = TestEgress::default();
    let attempt = core.forward_unicast_with_rejection(request, &mut scratch, &mut egress);

    assert_eq!(attempt.result, Err(ForwardError::NoRoute));
    let rejection = attempt.rejection.expect("valid source gets a refusal");
    assert_eq!(rejection.reason as u8, 1);
    assert_eq!(rejection.network, 200);
    assert_eq!(rejection.source.network().get(), 77);
    assert_eq!(rejection.source.address().as_slice(), &[9, 10]);
    let mut output = [0u8; 32];
    let length = rejection.encode_npdu(&mut output).unwrap();
    assert_eq!(
        &output[..length],
        &[0x01, 0xa0, 0x00, 0x4d, 0x02, 0x09, 0x0a, 0xff, 0x03, 0x01, 0x00, 0xc8,]
    );
    assert_eq!(egress.calls, 0);
    assert_eq!(core.diagnostics().drops_no_route, 1);
}

#[test]
fn oversize_refusal_uses_ingress_source_and_queue_is_not_called() {
    let mut core = configured_core();
    let ingress_source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let request = DirectForwardRequest {
        ingress_network: NetworkNumber::new(100).unwrap(),
        logical_source: ingress_source,
        npdu: &routed_npdu(498),
        egress_capabilities: capabilities(),
        now: 0,
    };
    let mut scratch = [0u8; 501];
    let mut egress = TestEgress::default();
    let attempt = core.forward_direct_with_rejection(request, &mut scratch, &mut egress);

    assert_eq!(
        attempt.result,
        Ok(ForwardOutcome::Oversize {
            final_npdu_len: 502,
            maximum: 501,
        })
    );
    let rejection = attempt.rejection.expect("oversize is a network refusal");
    assert_eq!(rejection.reason as u8, 4);
    assert_eq!(rejection.source.network(), NetworkNumber::new(100).unwrap());
    assert_eq!(rejection.source.address(), ingress_source);
    let mut output = [0u8; 16];
    let length = rejection.encode_npdu(&mut output).unwrap();
    assert_eq!(&output[..length], &[0x01, 0x80, 0x03, 0x04, 0x00, 0xc8]);
    assert_eq!(egress.calls, 0);
    assert_eq!(core.diagnostics().drops_oversize, 1);
}

#[test]
fn queue_full_refusal_is_busy_and_counts_only_the_forwarding_drop() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 512];
    let mut egress = TestEgress {
        outcome: Some(AdmissionOutcome::Full),
        ..TestEgress::default()
    };
    let attempt = core.forward_direct_with_rejection(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &routed_npdu(100),
            egress_capabilities: capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    );

    assert_eq!(attempt.result, Ok(ForwardOutcome::QueueFull));
    assert_eq!(
        attempt.rejection.map(|rejection| rejection.reason as u8),
        Some(2)
    );
    assert_eq!(egress.calls, 1);
    assert_eq!(core.diagnostics().drops_full, 1);
    assert_eq!(core.diagnostics().drops_no_route, 0);
    assert_eq!(core.diagnostics().drops_invalid, 0);
}

#[test]
fn unknown_route_requests_who_is_router_then_times_out_to_a_source_refusal() {
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let input = [0x01, 0x20, 0x01, 0x2c, 0x01, 0x09, 0xff, 0x10];
    let mut core = RouterCore::<2, 2>::new();
    let mut discovery = RouteDiscovery::<1>::new(10);
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    let request = |now| DirectForwardRequest {
        ingress_network: NetworkNumber::new(100).unwrap(),
        logical_source: source,
        npdu: &input,
        egress_capabilities: capabilities(),
        now,
    };

    let first = core.forward_unicast_with_discovery(
        request(100),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert_eq!(first.result, Err(ForwardError::NoRoute));
    assert_eq!(
        first.discovery,
        Some(DiscoveryAction::WhoIsRouter(
            bacnet_router_core::DiscoveryRequest {
                network: NetworkNumber::new(300).unwrap(),
                deadline: 110,
            },
        ))
    );
    assert!(first.rejection.is_none());

    let pending = core.forward_unicast_with_discovery(
        request(105),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert!(matches!(
        pending.discovery,
        Some(DiscoveryAction::Pending(_))
    ));
    assert!(pending.rejection.is_none());

    let timed_out = core.forward_unicast_with_discovery(
        request(110),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert_eq!(timed_out.result, Err(ForwardError::NoRoute));
    assert!(timed_out.discovery.is_none());
    assert_eq!(
        timed_out.rejection.map(|reject| reject.reason as u8),
        Some(1)
    );
    assert_eq!(discovery.pending_len(), 0);
    assert_eq!(egress.calls, 0);
}

#[test]
fn zero_hop_discovery_attempt_is_dropped_without_action_or_refusal() {
    let mut core = RouterCore::<2, 2>::new();
    let mut discovery = RouteDiscovery::<1>::new(10);
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    let input = [0x01, 0x20, 0x01, 0x2c, 0x01, 0x09, 0x00, 0x10];
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let attempt = core.forward_unicast_with_discovery(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &input,
            egress_capabilities: capabilities(),
            now: 100,
        },
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert_eq!(attempt.result, Err(ForwardError::HopExhausted));
    assert!(attempt.discovery.is_none());
    assert!(attempt.rejection.is_none());
    assert_eq!(discovery.pending_len(), 0);
    assert_eq!(egress.calls, 0);
}
