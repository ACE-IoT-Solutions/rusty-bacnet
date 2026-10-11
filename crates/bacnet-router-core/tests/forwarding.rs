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
}

impl Egress for TestEgress {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
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

    let mut broadcast_mac = routed_npdu(12);
    broadcast_mac[5] = 0xff;
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
    )
    .unwrap();
    assert_eq!(egress.destination, Some(EgressDestination::Broadcast));
}
