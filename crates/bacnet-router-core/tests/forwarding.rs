//! Direct forwarding, final-size, and actual-admission tests.

use bacnet_router_core::{
    decode_bip_datagram, encode_forwarded_npdu, AdmissionOutcome, DirectForwardRequest, Egress,
    EgressDestination, EgressRequest, ForwardError, ForwardOutcome, Ipv4SocketAddress, LinkAddress,
    NetworkNumber, PortCapabilities, PortId, PortKind, RouterCore, UdpDatagramMeta,
};

#[derive(Default)]
struct TestEgress {
    outcome: Option<AdmissionOutcome>,
    accepted: Vec<u8>,
    priority: Option<u8>,
    destination: Option<EgressDestination>,
    calls: usize,
}

impl Egress for TestEgress {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        self.calls += 1;
        let outcome = self.outcome.unwrap_or(AdmissionOutcome::AcceptedOwned);
        if outcome == AdmissionOutcome::AcceptedOwned {
            self.accepted.clear();
            self.accepted.extend_from_slice(request.npdu);
            self.priority = Some(request.priority);
            self.destination = Some(request.destination);
        }
        outcome
    }
}

fn routed_npdu(total_len: usize) -> Vec<u8> {
    assert!(total_len >= 7);
    let mut npdu = vec![0x5a; total_len];
    npdu[..7].copy_from_slice(&[
        0x01, // version
        0x23, // DNET present, life-safety priority
        0x00, 0xc8, // DNET 200
        0x01, 0x05, // one-byte MS/TP DADR
        0xff, // hop count
    ]);
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

fn learned_capabilities() -> PortCapabilities {
    PortCapabilities {
        id: PortId(3),
        network: NetworkNumber::new(200).unwrap(),
        kind: PortKind::Bip,
        max_npdu_len: 1476,
    }
}

fn learned_core() -> RouterCore<2, 2> {
    let mut core = RouterCore::new();
    core.routes_mut().learn(
        NetworkNumber::new(300).unwrap(),
        PortId(3),
        LinkAddress::try_from_slice(&[192, 0, 2, 1, 0xba, 0xc1]).unwrap(),
        10,
    );
    core
}

#[test]
fn final_rewritten_npdu_exactly_at_standard_mstp_limit_is_owned() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 501];
    let mut egress = TestEgress::default();

    let outcome = core
        .forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(497),
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(
        outcome,
        ForwardOutcome::Forwarded {
            final_npdu_len: 501
        }
    );
    assert_eq!(egress.accepted.len(), 501);
    assert_eq!(
        &egress.accepted[..11],
        &[0x01, 0x0b, 0x00, 0x64, 0x06, 192, 0, 2, 10, 0xba, 0xc1]
    );
    assert_eq!(egress.priority, Some(3));
    assert_eq!(
        egress.destination,
        Some(EgressDestination::Unicast(
            LinkAddress::try_from_slice(&[5]).unwrap()
        ))
    );
    assert_eq!(core.diagnostics().forwarded_npdus, 1);
}

#[test]
fn final_rewritten_npdu_one_over_limit_never_reaches_queue() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 501];
    let mut egress = TestEgress::default();

    let outcome = core
        .forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(498),
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(
        outcome,
        ForwardOutcome::Oversize {
            final_npdu_len: 502,
            maximum: 501,
        }
    );
    assert!(egress.accepted.is_empty());
    assert_eq!(core.diagnostics().drops_oversize, 1);
}

#[test]
fn actual_egress_queue_full_is_not_reported_as_forwarded() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 512];
    let mut egress = TestEgress {
        outcome: Some(AdmissionOutcome::Full),
        ..TestEgress::default()
    };

    let outcome = core
        .forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(100),
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(outcome, ForwardOutcome::QueueFull);
    assert_eq!(core.diagnostics().forwarded_npdus, 0);
    assert_eq!(core.diagnostics().drops_full, 1);
}

#[test]
fn actual_egress_expiry_and_invalid_request_remain_distinct() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 512];
    let mut core = configured_core();
    for (admission, expected) in [
        (AdmissionOutcome::Expired, ForwardOutcome::Expired),
        (AdmissionOutcome::Invalid, ForwardOutcome::EgressRejected),
    ] {
        let mut egress = TestEgress {
            outcome: Some(admission),
            ..TestEgress::default()
        };
        let outcome = core
            .forward_unicast(
                DirectForwardRequest {
                    ingress_network: NetworkNumber::new(100).unwrap(),
                    logical_source: source,
                    npdu: &routed_npdu(100),
                    egress_capabilities: capabilities(),
                    now: 0,
                },
                &mut scratch,
                &mut egress,
            )
            .unwrap();
        assert_eq!(outcome, expected);
    }
    assert_eq!(core.diagnostics().drops_expired, 1);
    assert_eq!(core.diagnostics().drops_invalid, 1);
}

#[test]
fn zero_hop_and_truncated_proprietary_network_message_fail_closed() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    let mut zero_hop = routed_npdu(12);
    zero_hop[6] = 0;
    let mut core = configured_core();
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &zero_hop,
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::HopExhausted)
    );

    let proprietary_without_vendor = [0x01, 0xa0, 0x00, 0xc8, 0x01, 0x05, 0xff, 0x80];
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &proprietary_without_vendor,
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::InvalidNetworkMessage)
    );
    assert!(egress.accepted.is_empty());
    assert_eq!(core.diagnostics().drops_invalid, 2);
}

#[test]
fn direct_forwarding_rejects_ingress_loop_and_wrong_mstp_mac_width() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    let mut looped = configured_core();
    assert_eq!(
        looped.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(200).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(12),
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::RouteLoopsToIngress)
    );

    let wrong_width = [
        0x01, 0x20, 0x00, 0xc8, 0x06, 192, 0, 2, 20, 0xba, 0xc0, 0xff, 0x10,
    ];
    let mut core = configured_core();
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &wrong_width,
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::InvalidEgressAddress)
    );
    assert!(egress.accepted.is_empty());
}

#[test]
fn forwarded_bip_origin_not_bbmd_peer_becomes_routed_sadr() {
    let origin = Ipv4SocketAddress {
        ip: [198, 51, 100, 7],
        port: 0xbac1,
    };
    let peer = Ipv4SocketAddress {
        ip: [192, 0, 2, 9],
        port: 0xbac0,
    };
    let npdu = routed_npdu(12);
    let mut datagram = [0u8; 64];
    let datagram_len = encode_forwarded_npdu(&mut datagram, origin, &npdu).unwrap();
    let ingress = decode_bip_datagram(
        &datagram[..datagram_len],
        UdpDatagramMeta {
            peer,
            destination: Some([192, 0, 2, 20]),
            destination_is_group: false,
            truncated: false,
        },
    )
    .unwrap();
    assert_ne!(ingress.logical_source_mac(), ingress.immediate_peer_mac());

    let mut core = configured_core();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    core.forward_direct(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: ingress.logical_source_mac(),
            npdu: ingress.npdu,
            egress_capabilities: capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();

    assert_eq!(
        &egress.accepted[..11],
        &[0x01, 0x0b, 0x00, 0x64, 0x06, 198, 51, 100, 7, 0xba, 0xc1]
    );
}

#[test]
fn direct_rewrite_preserves_existing_source_and_network_message_header() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut core = configured_core();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    let existing_source = [
        0x01, 0x28, // version, destination + source
        0x00, 0xc8, 0x01, 0x05, // destination
        0x01, 0x2c, 0x01, 0x09, // SNET 300, SADR 9
        0xff, 0x10, 0x22, // hop and APDU bytes
    ];
    core.forward_direct(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &existing_source,
            egress_capabilities: capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();
    assert_eq!(
        egress.accepted,
        [0x01, 0x08, 0x01, 0x2c, 0x01, 0x09, 0x10, 0x22]
    );

    let proprietary = [
        0x01, 0xa0, // network message + destination
        0x00, 0xc8, 0x01, 0x05, 0xff, // destination + hop
        0x80, 0x12, 0x34, 0xaa, // type, vendor, payload
    ];
    core.forward_direct(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &proprietary,
            egress_capabilities: capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();
    assert_eq!(
        egress.accepted,
        [0x01, 0x88, 0x00, 0x64, 0x06, 192, 0, 2, 10, 0xba, 0xc1, 0x80, 0x12, 0x34, 0xaa,]
    );
}

#[test]
fn learned_unicast_preserves_final_dadr_decrements_hop_and_uses_router_next_hop() {
    let mut core = learned_core();
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let input = [
        0x01, 0x20, // version, destination present
        0x01, 0x2c, 0x06, 192, 0, 2, 44, 0xba, 0xc1, // DNET/DADR
        0x02, // hop count
        0x10, 0x22, // APDU
    ];
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    let outcome = core
        .forward_unicast(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &input,
                egress_capabilities: learned_capabilities(),
                now: 11,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(outcome, ForwardOutcome::Forwarded { final_npdu_len: 23 });
    assert_eq!(
        &egress.accepted,
        &[
            0x01, 0x28, // destination + source
            0x01, 0x2c, 0x06, 192, 0, 2, 44, 0xba, 0xc1, // final DNET/DADR
            0x00, 0x64, 0x06, 198, 51, 100, 7, 0xba, 0xc1, // inserted SNET/SADR
            0x01, // 2 -> 1
            0x10, 0x22,
        ]
    );
    assert_eq!(
        egress.destination,
        Some(EgressDestination::Unicast(
            LinkAddress::try_from_slice(&[192, 0, 2, 1, 0xba, 0xc1]).unwrap()
        ))
    );
}

#[test]
fn learned_unicast_preserves_source_and_proprietary_network_message_bytes() {
    let mut core = learned_core();
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let input = [
        0x01, 0xa8, // destination + source + network layer message
        0x01, 0x2c, 0x01, 0x09, // DNET/DADR
        0x00, 0xc8, 0x01, 0x05, // existing SNET/SADR
        0xff, // hop count
        0x80, 0x12, 0x34, // proprietary type + vendor id
        0xde, 0xad,
    ];
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    core.forward_unicast(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &input,
            egress_capabilities: learned_capabilities(),
            now: 11,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();

    assert_eq!(
        &egress.accepted,
        &[
            0x01, 0xa8, // control preserved
            0x01, 0x2c, 0x01, 0x09, 0x00, 0xc8, 0x01, 0x05, // source preserved
            0xfe, // 255 -> 254
            0x80, 0x12, 0x34, 0xde, 0xad, // network message/APDU preserved
        ]
    );
}

#[test]
fn learned_unicast_distinguishes_no_route_and_busy_route() {
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let input = [0x01, 0x20, 0x01, 0x90, 0x01, 0x09, 0xff, 0x10];
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    let mut core = RouterCore::<2, 2>::new();

    assert_eq!(
        core.forward_unicast(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &input,
                egress_capabilities: PortCapabilities {
                    id: PortId(3),
                    network: NetworkNumber::new(400).unwrap(),
                    kind: PortKind::Bip,
                    max_npdu_len: 1476,
                },
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::NoRoute)
    );

    let next_hop = LinkAddress::try_from_slice(&[192, 0, 2, 1, 0xba, 0xc1]).unwrap();
    core.routes_mut()
        .learn(NetworkNumber::new(400).unwrap(), PortId(3), next_hop, 0);
    assert!(core
        .routes_mut()
        .mark_busy(NetworkNumber::new(400).unwrap(), 5));
    assert_eq!(
        core.forward_unicast(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &input,
                egress_capabilities: PortCapabilities {
                    id: PortId(3),
                    network: NetworkNumber::new(400).unwrap(),
                    kind: PortKind::Bip,
                    max_npdu_len: 1476,
                },
                now: 4,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::RouteUnavailable)
    );
    assert_eq!(
        core.routes()
            .lookup(NetworkNumber::new(400).unwrap())
            .unwrap()
            .next_hop,
        next_hop
    );
}

#[test]
fn learned_zero_and_one_hop_are_rejected_before_egress() {
    let mut core = learned_core();
    let source = LinkAddress::try_from_slice(&[198, 51, 100, 7, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    for hop in [0, 1] {
        let input = [
            0x01, 0x20, 0x01, 0x2c, 0x06, 192, 0, 2, 44, 0xba, 0xc1, hop, 0x10,
        ];
        assert_eq!(
            core.forward_unicast(
                DirectForwardRequest {
                    ingress_network: NetworkNumber::new(100).unwrap(),
                    logical_source: source,
                    npdu: &input,
                    egress_capabilities: learned_capabilities(),
                    now: 11,
                },
                &mut scratch,
                &mut egress,
            ),
            Err(ForwardError::HopExhausted)
        );
    }
    assert!(egress.accepted.is_empty());
}

#[test]
fn standard_mstp_hard_limit_and_broadcast_mac_are_enforced() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut core = configured_core();
    let mut scratch = [0u8; 502];
    let mut egress = TestEgress::default();
    let mut overstated = capabilities();
    overstated.max_npdu_len = 1497;
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(498),
                egress_capabilities: overstated,
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap(),
        ForwardOutcome::Oversize {
            final_npdu_len: 502,
            maximum: 501,
        }
    );

    let calls_before_broadcast = egress.calls;
    let mut broadcast_mac = routed_npdu(12);
    broadcast_mac[5] = 0xff;
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &broadcast_mac,
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::BroadcastRequiresPolicy)
    );
    assert_eq!(egress.calls, calls_before_broadcast);
}

#[test]
fn malformed_input_has_no_refusal_and_never_reaches_egress() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();
    let attempt = core.forward_direct_with_rejection(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &[0x01, 0x23, 0x00],
            egress_capabilities: capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    );
    assert_eq!(attempt.result, Err(ForwardError::Truncated));
    assert!(attempt.rejection.is_none());
    assert_eq!(egress.calls, 0);
}

#[test]
fn zero_and_one_hop_are_silently_discarded_without_a_refusal() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 128];
    let mut egress = TestEgress::default();
    for hop in [0, 1] {
        let mut npdu = routed_npdu(100);
        npdu[6] = hop;
        let attempt = core.forward_direct_with_rejection(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &npdu,
                egress_capabilities: capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        );
        assert_eq!(attempt.result, Err(ForwardError::HopExhausted));
        assert!(attempt.rejection.is_none());
    }
    assert_eq!(egress.calls, 0);
}
