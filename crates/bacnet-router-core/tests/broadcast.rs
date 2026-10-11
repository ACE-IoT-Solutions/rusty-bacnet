//! Routed-broadcast policy, fanout, and loop-suppression tests.

use bacnet_router_core::{
    AdmissionOutcome, BroadcastDestination, BroadcastFanoutOutcome, BroadcastFanoutResult,
    BroadcastPolicy, BroadcastPolicyConfig, BroadcastRequest, BroadcastTarget, Egress,
    EgressRequest, LinkAddress, NetworkNumber, PortCapabilities, PortId, PortKind, TrafficClass,
};

#[derive(Default)]
struct RecordingEgress {
    calls: Vec<(PortId, Vec<u8>)>,
}

impl Egress for RecordingEgress {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        self.calls.push((request.port, request.npdu.to_vec()));
        match request.port {
            PortId(3) => AdmissionOutcome::Full,
            PortId(4) => AdmissionOutcome::PortDown,
            PortId(5) => AdmissionOutcome::Oversize,
            PortId(6) => AdmissionOutcome::Invalid,
            _ => AdmissionOutcome::AcceptedOwned,
        }
    }
}

fn network(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

fn address(value: u8) -> LinkAddress {
    LinkAddress::try_from_slice(&[value]).unwrap()
}

fn target(port: u8, network_number: u16, maximum: usize) -> BroadcastTarget {
    BroadcastTarget {
        capabilities: PortCapabilities {
            id: PortId(port),
            network: network(network_number),
            kind: PortKind::MstpStandard,
            max_npdu_len: maximum,
        },
    }
}

fn request<'a>(
    ingress_port: u8,
    destination: BroadcastDestination,
    npdu: &'a [u8],
    fingerprint: u64,
    now: u64,
) -> BroadcastRequest<'a> {
    BroadcastRequest {
        ingress_port: PortId(ingress_port),
        origin_network: network(10),
        origin_address: address(0x42),
        destination,
        npdu,
        hop_count_offset: Some(3),
        packet_fingerprint: fingerprint,
        class: TrafficClass::Data,
        priority: 2,
        now,
    }
}

fn global_policy<const CAPACITY: usize>(retention: u64) -> BroadcastPolicy<CAPACITY> {
    BroadcastPolicy::new(BroadcastPolicyConfig {
        allow_remote: true,
        allow_global: true,
        echo_retention: retention,
    })
}

fn results(len: usize) -> Vec<BroadcastFanoutResult> {
    vec![
        BroadcastFanoutResult {
            port: PortId(0),
            outcome: BroadcastFanoutOutcome::PolicyDisabled,
        };
        len
    ]
}

#[test]
fn default_policy_is_fail_closed_and_remote_selects_only_its_network() {
    let npdu = [1, 0, 0, 5, 0xaa];
    let targets = [
        target(1, 100, 501),
        target(2, 200, 501),
        target(3, 300, 501),
    ];
    let mut output = results(targets.len());
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = BroadcastPolicy::<4>::default();

    let report = policy
        .forward(
            request(1, BroadcastDestination::Remote(network(200)), &npdu, 1, 0),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    assert_eq!(report.selected, 0);
    assert!(egress.calls.is_empty());
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::IngressExcluded);
    assert_eq!(output[1].outcome, BroadcastFanoutOutcome::PolicyDisabled);
}

#[test]
fn global_fanout_excludes_ingress_preserves_mixed_outcomes_and_decrements_once() {
    let npdu = [1, 0, 0, 5, 0xaa];
    let targets = [
        target(1, 100, 501),
        target(2, 200, 501),
        target(3, 300, 501),
        target(4, 400, 501),
        target(5, 500, 3),
        target(6, 600, 501),
    ];
    let mut output = results(targets.len());
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<8>(100);

    let report = policy
        .forward(
            request(1, BroadcastDestination::Global, &npdu, 1, 10),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(report.selected, 5);
    assert_eq!(report.accepted, 1);
    assert_eq!(report.full, 1);
    assert_eq!(report.port_down, 1);
    assert_eq!(report.oversize, 1);
    assert_eq!(report.invalid, 1);
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::IngressExcluded);
    assert!(matches!(
        output[1].outcome,
        BroadcastFanoutOutcome::Accepted { .. }
    ));
    assert_eq!(output[2].outcome, BroadcastFanoutOutcome::Full);
    assert_eq!(output[3].outcome, BroadcastFanoutOutcome::PortDown);
    assert!(matches!(
        output[4].outcome,
        BroadcastFanoutOutcome::Oversize { .. }
    ));
    assert_eq!(output[5].outcome, BroadcastFanoutOutcome::Invalid);
    assert_eq!(egress.calls[0].1[3], 4);
}

#[test]
fn alternate_ingress_is_suppressed_but_repeat_after_expiry_is_admitted() {
    let npdu = [1, 0, 0, 2, 0xaa];
    let targets = [target(1, 100, 501), target(2, 200, 501)];
    let mut output = results(targets.len());
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(10);

    policy
        .forward(
            request(1, BroadcastDestination::Global, &npdu, 9, 0),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    policy
        .forward(
            request(7, BroadcastDestination::Global, &npdu, 9, 5),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::Duplicate);
    assert_eq!(output[1].outcome, BroadcastFanoutOutcome::Duplicate);

    policy
        .forward(
            request(7, BroadcastDestination::Global, &npdu, 9, 10),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    assert_eq!(
        output[0].outcome,
        BroadcastFanoutOutcome::Accepted { final_npdu_len: 5 }
    );
    assert_eq!(
        output[1].outcome,
        BroadcastFanoutOutcome::Accepted { final_npdu_len: 5 }
    );
    assert_eq!(egress.calls.len(), 3);
}

#[test]
fn cache_saturation_replaces_deterministically_and_remains_bounded() {
    let npdu = [1, 0, 0, 4];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<2>(100);

    for fingerprint in 1..=3 {
        let report = policy
            .forward(
                request(1, BroadcastDestination::Global, &npdu, fingerprint, 0),
                &targets,
                &mut output,
                &mut scratch,
                &mut egress,
            )
            .unwrap();
        assert_eq!(report.accepted, 1);
    }
    assert_eq!(policy.cache_stats().occupancy, 2);
    assert_eq!(policy.cache_stats().evictions, 1);
}

#[test]
fn hopless_control_is_forwarded_once_and_alternate_ingress_cannot_loop_it() {
    let npdu = [1, 0x80, 0, 0x01];
    let targets = [
        target(1, 100, 501),
        target(2, 200, 501),
        target(3, 300, 501),
    ];
    let mut output = results(targets.len());
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(20);

    let mut first = request(1, BroadcastDestination::Global, &npdu, 99, 1);
    first.hop_count_offset = None;
    first.class = TrafficClass::NetworkControl;
    policy
        .forward(first, &targets, &mut output, &mut scratch, &mut egress)
        .unwrap();
    assert_eq!(egress.calls.len(), 2);

    let mut looped = request(2, BroadcastDestination::Global, &npdu, 99, 2);
    looped.hop_count_offset = None;
    looped.class = TrafficClass::NetworkControl;
    policy
        .forward(looped, &targets, &mut output, &mut scratch, &mut egress)
        .unwrap();
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::Duplicate);
    assert_eq!(output[1].outcome, BroadcastFanoutOutcome::IngressExcluded);
    assert_eq!(output[2].outcome, BroadcastFanoutOutcome::Duplicate);
    assert_eq!(egress.calls.len(), 2);
}
