//! Routed-broadcast policy, fanout, and loop-suppression tests.

use bacnet_router_core::{
    AdmissionOutcome, BroadcastDestination, BroadcastError, BroadcastFanoutOutcome,
    BroadcastFanoutResult, BroadcastPolicy, BroadcastPolicyConfig, BroadcastRequest,
    BroadcastTarget, Egress, EgressRequest, LinkAddress, NetworkNumber, PortCapabilities, PortId,
    PortKind, TrafficClass,
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
        hop_count_offset: if npdu.get(1).is_some_and(|control| control & 0x20 != 0) {
            Some(5)
        } else {
            None
        },
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
    let npdu = [1, 0x20, 0, 200, 0, 5, 0xaa];
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
fn malformed_or_undersized_final_npdu_is_rejected_before_cache_or_egress() {
    let targets = [target(2, 200, 10)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    let malformed = [1, 0x20, 0, 200, 0];
    assert_eq!(
        policy.forward(
            request(
                1,
                BroadcastDestination::Remote(network(200)),
                &malformed,
                1,
                1,
            ),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        ),
        Err(BroadcastError::InvalidNpdu)
    );

    let routed = [1, 0x20, 0xff, 0xff, 0, 5, 0xaa];
    let mut small_scratch = [0; 10];
    assert_eq!(
        policy.forward(
            request(1, BroadcastDestination::Global, &routed, 2, 1),
            &targets,
            &mut output,
            &mut small_scratch,
            &mut egress,
        ),
        Err(BroadcastError::ScratchTooSmall {
            needed: 11,
            available: 10,
        })
    );
    assert!(egress.calls.is_empty());
}

#[test]
fn global_validation_rejects_nonbroadcast_dadr_local_apdu_and_class_mismatch() {
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 32];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    let nonbroadcast_dadr = [1, 0x20, 0xff, 0xff, 1, 5, 5, 0xaa];
    let mut nonbroadcast = request(1, BroadcastDestination::Global, &nonbroadcast_dadr, 1, 0);
    nonbroadcast.hop_count_offset = Some(6);
    assert_eq!(
        policy.forward(
            nonbroadcast,
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        ),
        Err(BroadcastError::InvalidNpdu)
    );

    let local_apdu = [1, 0x00, 0x10, 0x08];
    assert_eq!(
        policy.forward(
            request(1, BroadcastDestination::Global, &local_apdu, 2, 0),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        ),
        Err(BroadcastError::InvalidNpdu)
    );

    let global = [1, 0x20, 0xff, 0xff, 0, 5, 0xaa];
    let mut wrong_class = request(1, BroadcastDestination::Global, &global, 3, 0);
    wrong_class.class = TrafficClass::NetworkControl;
    assert_eq!(
        policy.forward(
            wrong_class,
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        ),
        Err(BroadcastError::InvalidNpdu)
    );
    assert!(egress.calls.is_empty());
}

#[test]
fn global_hop_one_is_discarded_before_cache_or_egress() {
    let npdu = [1, 0x20, 0xff, 0xff, 0, 1, 0xaa];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    let report = policy
        .forward(
            request(1, BroadcastDestination::Global, &npdu, 1, 0),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    assert_eq!(report.selected, 1);
    assert_eq!(report.cache_outcome, None);
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::HopExhausted);
    assert!(egress.calls.is_empty());
}

#[test]
fn remote_hop_one_is_discarded_before_cache_or_egress() {
    let npdu = [1, 0x20, 0, 200, 0, 1, 0xaa];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    let report = policy
        .forward(
            request(1, BroadcastDestination::Remote(network(200)), &npdu, 1, 0),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();
    assert_eq!(report.selected, 1);
    assert_eq!(report.cache_outcome, None);
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::HopExhausted);
    assert!(egress.calls.is_empty());
}

#[test]
fn global_fanout_excludes_ingress_preserves_mixed_outcomes_and_decrements_once() {
    let npdu = [1, 0x20, 0xff, 0xff, 0, 5, 0xaa];
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
    assert_eq!(egress.calls[0].1[9], 4);
}

#[test]
fn remote_broadcast_strips_route_header_and_inserts_origin_exactly() {
    let npdu = [1, 0x20, 0, 200, 0, 5, 0xaa];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    policy
        .forward(
            request(1, BroadcastDestination::Remote(network(200)), &npdu, 7, 1),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(egress.calls[0].0, PortId(2));
    assert_eq!(egress.calls[0].1, [1, 0x08, 0, 10, 1, 0x42, 0xaa]);
}

#[test]
fn remote_broadcast_preserves_existing_origin_exactly() {
    let npdu = [1, 0x28, 0, 200, 0, 0, 20, 1, 9, 5, 0xaa];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);
    let mut request = request(1, BroadcastDestination::Remote(network(200)), &npdu, 17, 1);
    request.hop_count_offset = Some(9);

    policy
        .forward(request, &targets, &mut output, &mut scratch, &mut egress)
        .unwrap();

    assert_eq!(egress.calls[0].1, [1, 0x08, 0, 20, 1, 9, 0xaa]);
}

#[test]
fn global_broadcast_preserves_route_header_inserts_origin_and_decrements_once() {
    let npdu = [1, 0x20, 0xff, 0xff, 0, 5, 0xaa];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<4>(100);

    policy
        .forward(
            request(1, BroadcastDestination::Global, &npdu, 8, 1),
            &targets,
            &mut output,
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(
        egress.calls[0].1,
        [1, 0x28, 0xff, 0xff, 0, 0, 10, 1, 0x42, 4, 0xaa]
    );
}

#[test]
fn alternate_ingress_is_suppressed_but_repeat_after_expiry_is_admitted() {
    let npdu = [1, 0x20, 0xff, 0xff, 0, 2, 0xaa];
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
        BroadcastFanoutOutcome::Accepted { final_npdu_len: 11 }
    );
    assert_eq!(
        output[1].outcome,
        BroadcastFanoutOutcome::Accepted { final_npdu_len: 11 }
    );
    assert_eq!(egress.calls.len(), 3);
}

#[test]
fn cache_saturation_replaces_deterministically_and_remains_bounded() {
    let npdu = [1, 0x20, 0xff, 0xff, 0, 4];
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

#[test]
fn hopless_cache_saturation_fails_closed_without_a_b_c_churn_loop() {
    let npdu = [1, 0x80, 0, 0x01];
    let targets = [target(2, 200, 501)];
    let mut output = results(1);
    let mut scratch = [0; 16];
    let mut egress = RecordingEgress::default();
    let mut policy = global_policy::<2>(100);

    for fingerprint in 1..=2 {
        let mut control = request(1, BroadcastDestination::Global, &npdu, fingerprint, 1);
        control.class = TrafficClass::NetworkControl;
        policy
            .forward(control, &targets, &mut output, &mut scratch, &mut egress)
            .unwrap();
        assert_eq!(
            output[0].outcome,
            BroadcastFanoutOutcome::Accepted { final_npdu_len: 8 }
        );
    }
    let mut third = request(1, BroadcastDestination::Global, &npdu, 3, 1);
    third.class = TrafficClass::NetworkControl;
    policy
        .forward(third, &targets, &mut output, &mut scratch, &mut egress)
        .unwrap();
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::CacheUntracked);
    let mut repeated = request(1, BroadcastDestination::Global, &npdu, 1, 1);
    repeated.class = TrafficClass::NetworkControl;
    policy
        .forward(repeated, &targets, &mut output, &mut scratch, &mut egress)
        .unwrap();
    assert_eq!(output[0].outcome, BroadcastFanoutOutcome::CacheUntracked);
    assert_eq!(egress.calls.len(), 2);
}
