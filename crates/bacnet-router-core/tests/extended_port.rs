//! Extended MS/TP router-port limits and addressing tests.

use bacnet_router_core::{
    AdmissionOutcome, DirectForwardRequest, Egress, EgressDestination, EgressRequest, ForwardError,
    ForwardOutcome, LinkAddress, NetworkNumber, PortCapabilities, PortId, PortKind, RouterCore,
};

#[derive(Default)]
struct TestEgress {
    outcome: Option<AdmissionOutcome>,
    accepted: Vec<u8>,
    destination: Option<EgressDestination>,
}

impl Egress for TestEgress {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        let outcome = self.outcome.unwrap_or(AdmissionOutcome::AcceptedOwned);
        if outcome == AdmissionOutcome::AcceptedOwned {
            self.accepted.clear();
            self.accepted.extend_from_slice(request.npdu);
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

fn extended_capabilities() -> PortCapabilities {
    PortCapabilities {
        id: PortId(2),
        network: NetworkNumber::new(200).unwrap(),
        kind: PortKind::MstpExtended,
        max_npdu_len: 1497,
    }
}

#[test]
fn final_rewritten_npdu_exactly_at_extended_mstp_limit_is_owned() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 1497];
    let mut egress = TestEgress::default();

    let outcome = core
        .forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(1493),
                egress_capabilities: extended_capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(
        outcome,
        ForwardOutcome::Forwarded {
            final_npdu_len: 1497
        }
    );
    assert_eq!(egress.accepted.len(), 1497);
}

#[test]
fn final_rewritten_npdu_one_over_extended_mstp_limit_never_reaches_queue() {
    let mut core = configured_core();
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut scratch = [0u8; 1498];
    let mut egress = TestEgress::default();

    let outcome = core
        .forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &routed_npdu(1494),
                egress_capabilities: extended_capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        )
        .unwrap();

    assert_eq!(
        outcome,
        ForwardOutcome::Oversize {
            final_npdu_len: 1498,
            maximum: 1497,
        }
    );
    assert!(egress.accepted.is_empty());
}

#[test]
fn extended_mstp_uses_one_byte_station_addresses_and_ff_broadcast() {
    let source = LinkAddress::try_from_slice(&[192, 0, 2, 10, 0xba, 0xc1]).unwrap();
    let mut core = configured_core();
    let mut scratch = [0u8; 64];
    let mut egress = TestEgress::default();

    let mut unicast = routed_npdu(12);
    unicast[5] = 7;
    core.forward_direct(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &unicast,
            egress_capabilities: extended_capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();
    assert_eq!(
        egress.destination,
        Some(EgressDestination::Unicast(
            LinkAddress::try_from_slice(&[7]).unwrap()
        ))
    );

    let mut broadcast = routed_npdu(12);
    broadcast[5] = 0xff;
    core.forward_direct(
        DirectForwardRequest {
            ingress_network: NetworkNumber::new(100).unwrap(),
            logical_source: source,
            npdu: &broadcast,
            egress_capabilities: extended_capabilities(),
            now: 0,
        },
        &mut scratch,
        &mut egress,
    )
    .unwrap();
    assert_eq!(egress.destination, Some(EgressDestination::Broadcast));

    let mut invalid = routed_npdu(12);
    invalid[4] = 2;
    assert_eq!(
        core.forward_direct(
            DirectForwardRequest {
                ingress_network: NetworkNumber::new(100).unwrap(),
                logical_source: source,
                npdu: &invalid,
                egress_capabilities: extended_capabilities(),
                now: 0,
            },
            &mut scratch,
            &mut egress,
        ),
        Err(ForwardError::InvalidEgressAddress)
    );
}
