//! Bounded unknown-route discovery state regression tests.

use bacnet_router_core::{
    AdmissionOutcome, DirectForwardRequest, DiscoveryAction, Egress, EgressRequest, ForwardError,
    LinkAddress, NetworkNumber, PortCapabilities, PortId, PortKind, RouteDiscovery, RouterCore,
};

#[derive(Default)]
struct NoEgress {
    calls: usize,
}

impl Egress for NoEgress {
    fn try_admit(&mut self, _request: EgressRequest<'_>) -> AdmissionOutcome {
        self.calls += 1;
        AdmissionOutcome::AcceptedOwned
    }
}

fn network(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

fn request(now: u64, npdu: &[u8]) -> DirectForwardRequest<'_> {
    DirectForwardRequest {
        ingress_network: network(100),
        logical_source: LinkAddress::try_from_slice(&[1]).unwrap(),
        npdu,
        egress_capabilities: PortCapabilities {
            id: PortId(2),
            network: network(200),
            kind: PortKind::MstpStandard,
            max_npdu_len: 501,
        },
        now,
    }
}

#[test]
fn zero_capacity_reports_the_unknown_network_without_refusal_or_egress() {
    let npdu_300 = [0x01, 0x20, 0x01, 0x2c, 0x01, 0x09, 0xff, 0x10];
    let mut core = RouterCore::<1, 1>::new();
    let mut discovery = RouteDiscovery::<0>::new(10);
    let mut egress = NoEgress::default();
    let mut scratch = [0; 32];

    let attempt = core.forward_unicast_with_discovery(
        request(1, &npdu_300),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );

    assert_eq!(attempt.result, Err(ForwardError::NoRoute));
    assert_eq!(
        attempt.discovery,
        Some(DiscoveryAction::CapacityFull {
            network: network(300),
        })
    );
    assert!(attempt.rejection.is_none());
    assert_eq!(discovery.pending_len(), 0);
    assert_eq!(egress.calls, 0);
}

#[test]
fn a_full_table_is_bounded_and_learning_releases_the_exact_slot() {
    let npdu_300 = [0x01, 0x20, 0x01, 0x2c, 0x01, 0x09, 0xff, 0x10];
    let npdu_301 = [0x01, 0x20, 0x01, 0x2d, 0x01, 0x09, 0xff, 0x10];
    let mut core = RouterCore::<1, 1>::new();
    let mut discovery = RouteDiscovery::<1>::new(10);
    let mut egress = NoEgress::default();
    let mut scratch = [0; 32];

    let first = core.forward_unicast_with_discovery(
        request(5, &npdu_300),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert!(matches!(
        first.discovery,
        Some(DiscoveryAction::WhoIsRouter(_))
    ));
    assert_eq!(discovery.pending_len(), 1);

    let full = core.forward_unicast_with_discovery(
        request(6, &npdu_301),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert_eq!(
        full.discovery,
        Some(DiscoveryAction::CapacityFull {
            network: network(301),
        })
    );
    assert!(full.rejection.is_none());
    assert_eq!(discovery.pending_len(), 1);

    assert!(!discovery.route_learned(network(301)));
    assert!(discovery.route_learned(network(300)));
    assert_eq!(discovery.pending_len(), 0);

    let retried = core.forward_unicast_with_discovery(
        request(7, &npdu_300),
        &mut scratch,
        &mut egress,
        &mut discovery,
    );
    assert!(matches!(
        retried.discovery,
        Some(DiscoveryAction::WhoIsRouter(_))
    ));
    discovery.clear();
    assert_eq!(discovery.pending_len(), 0);
    assert_eq!(egress.calls, 0);
}
