//! Exactly-once diagnostics at the concrete adapter composition boundary.

#![allow(clippy::too_many_arguments)]

use bacnet_mstp_core::{
    Action, ActionBuffers, HeaplessQueues, Instant, MasterCore, MstpCoreConfig, RequestId,
};
use bacnet_router_adapters::{
    bip::{BipDatagramSink, BipEgress},
    composition::{
        forward_bip_datagram_to_mstp_observed, forward_mstp_action_to_bip_observed,
        BindingAdmission, CompositionError, ObservedCompositionError, RequestBindings,
    },
    mstp::MstpEgress,
};
use bacnet_router_core::{
    encode_forwarded_npdu, AdmissionOutcome, BipDecodeError, ForwardOutcome, Ipv4SocketAddress,
    NetworkNumber, OperationalDiagnostics, PortCapabilities, PortId, PortKind, RouterCore,
    UdpDatagramMeta,
};

type Queues = HeaplessQueues<1, 1>;
type Actions = ActionBuffers<1>;
type Diagnostics = OperationalDiagnostics<4, 4, 4>;

fn net(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

fn mstp_caps(port: u8, network: u16) -> PortCapabilities {
    PortCapabilities {
        id: PortId(port),
        network: net(network),
        kind: PortKind::MstpStandard,
        max_npdu_len: 501,
    }
}

fn bip_caps(port: u8, network: u16) -> PortCapabilities {
    PortCapabilities {
        id: PortId(port),
        network: net(network),
        kind: PortKind::Bip,
        max_npdu_len: 1476,
    }
}

fn new_diagnostics() -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    diagnostics.register_port(mstp_caps(2, 200)).unwrap();
    diagnostics.register_port(bip_caps(3, 300)).unwrap();
    diagnostics
}

fn new_mstp_core<'rx>(rx: &'rx mut [u8], actions: &'rx mut Actions) -> MasterCore<'rx, Queues> {
    MasterCore::new(
        MstpCoreConfig {
            station: 1,
            max_master: 2,
            ..MstpCoreConfig::default()
        },
        Queues::new(),
        rx,
        actions,
        Instant::ZERO,
    )
    .unwrap()
}

fn routed_npdu(total_len: usize, network: u16, destination: &[u8]) -> Vec<u8> {
    let mut npdu = vec![0x5a; total_len];
    npdu[0] = 1;
    npdu[1] = 0x23;
    npdu[2..4].copy_from_slice(&network.to_be_bytes());
    npdu[4] = destination.len() as u8;
    npdu[5..5 + destination.len()].copy_from_slice(destination);
    npdu[5 + destination.len()] = 0xff;
    npdu
}

fn metadata() -> UdpDatagramMeta {
    UdpDatagramMeta {
        peer: Ipv4SocketAddress {
            ip: [198, 51, 100, 2],
            port: 47_809,
        },
        destination: Some([192, 0, 2, 20]),
        destination_is_group: false,
        truncated: false,
    }
}

fn forwarded_datagram(npdu: &[u8]) -> Vec<u8> {
    let mut datagram = vec![0; 1600];
    let length = encode_forwarded_npdu(
        &mut datagram,
        Ipv4SocketAddress {
            ip: [198, 51, 100, 10],
            port: 47_809,
        },
        npdu,
    )
    .unwrap();
    datagram.truncate(length);
    datagram
}

#[derive(Clone, Copy)]
struct Sink {
    outcome: AdmissionOutcome,
    calls: usize,
}

impl Sink {
    const fn new(outcome: AdmissionOutcome) -> Self {
        Self { outcome, calls: 0 }
    }
}

impl BipDatagramSink for Sink {
    fn try_send(
        &mut self,
        _destination: Ipv4SocketAddress,
        _datagram: &[u8],
        _now: Instant,
        _class: bacnet_router_core::TrafficClass,
    ) -> AdmissionOutcome {
        self.calls += 1;
        self.outcome
    }
}

#[test]
fn malformed_bvll_is_preflighted_and_observed_once() {
    let mut router = RouterCore::<2, 2>::new();
    let mut diagnostics = new_diagnostics();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::ZERO);
    let mut scratch = [0; 512];

    let result = forward_bip_datagram_to_mstp_observed(
        &mut router,
        &mut diagnostics,
        &[0x81, 0x0a],
        metadata(),
        Instant::ZERO,
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    );

    assert_eq!(
        result,
        Err(ObservedCompositionError::Composition(
            CompositionError::BipDecode(BipDecodeError::HeaderTruncated),
        ))
    );
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.malformed, 1);
    assert_eq!(router.diagnostics().ingress_npdus, 0);
    assert_eq!(mstp.core_mut().counters().unicast.depth, 0);
}

#[test]
fn unregistered_egress_is_rejected_before_raw_composition() {
    let mut router = RouterCore::<2, 2>::new();
    let mut diagnostics = Diagnostics::new();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::ZERO);
    let mut scratch = [0; 512];

    let result = forward_bip_datagram_to_mstp_observed(
        &mut router,
        &mut diagnostics,
        &[],
        metadata(),
        Instant::ZERO,
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    );

    assert_eq!(
        result,
        Err(ObservedCompositionError::Diagnostics(
            bacnet_router_core::OperationalError::UnknownPort(PortId(2)),
        ))
    );
    assert_eq!(router.diagnostics().ingress_npdus, 0);
}

#[test]
fn real_mstp_success_full_oversize_expired_and_invalid_are_observed() {
    // Success and queue-full use one concrete bounded MS/TP queue.
    let npdu = routed_npdu(100, 200, &[5]);
    let datagram = forwarded_datagram(&npdu);
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(200), PortId(2)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::from_micros(1));
    let mut scratch = [0; 1600];

    assert!(matches!(
        forward_bip_datagram_to_mstp_observed(
            &mut router,
            &mut diagnostics,
            &datagram,
            metadata(),
            Instant::from_micros(1),
            net(100),
            mstp_caps(2, 200),
            &mut scratch,
            &mut mstp,
        ),
        Ok((_, ForwardOutcome::Forwarded { .. }))
    ));
    assert!(matches!(
        forward_bip_datagram_to_mstp_observed(
            &mut router,
            &mut diagnostics,
            &datagram,
            metadata(),
            Instant::from_micros(2),
            net(100),
            mstp_caps(2, 200),
            &mut scratch,
            &mut mstp,
        ),
        Ok((_, ForwardOutcome::QueueFull))
    ));
    let counters = diagnostics.port(PortId(2)).unwrap().1;
    assert_eq!(counters.accepted, 1);
    assert_eq!(counters.full, 1);

    // The standard concrete egress refuses a final NPDU over 501 bytes.
    let oversized = forwarded_datagram(&routed_npdu(500, 200, &[5]));
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(200), PortId(2)).unwrap();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::ZERO);
    let mut diagnostics = new_diagnostics();
    let outcome = forward_bip_datagram_to_mstp_observed(
        &mut router,
        &mut diagnostics,
        &oversized,
        metadata(),
        Instant::ZERO,
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    )
    .unwrap()
    .1;
    assert!(matches!(outcome, ForwardOutcome::Oversize { .. }));
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.oversize, 1);

    // A regressed adapter timestamp is an actual egress expiry.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(200), PortId(2)).unwrap();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::from_micros(2));
    mstp.set_now(Instant::from_micros(1));
    let mut diagnostics = new_diagnostics();
    let outcome = forward_bip_datagram_to_mstp_observed(
        &mut router,
        &mut diagnostics,
        &datagram,
        metadata(),
        Instant::from_micros(1),
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    )
    .unwrap()
    .1;
    assert_eq!(outcome, ForwardOutcome::Expired);
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.expired, 1);

    // A concrete egress bound to the wrong port returns Invalid; the outer
    // wrapper observes that terminal adapter result exactly once.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(200), PortId(2)).unwrap();
    let mut rx = [0; 1024];
    let mut actions = Actions::new();
    let mut mstp_core = new_mstp_core(&mut rx, &mut actions);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(99), Instant::ZERO);
    let mut diagnostics = new_diagnostics();
    let outcome = forward_bip_datagram_to_mstp_observed(
        &mut router,
        &mut diagnostics,
        &datagram,
        metadata(),
        Instant::ZERO,
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    )
    .unwrap()
    .1;
    assert_eq!(outcome, ForwardOutcome::EgressRejected);
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.malformed, 1);
}

fn reverse_action<'a>(npdu: &'a [u8], request_id: Option<RequestId>) -> Action<'a> {
    Action::DeliverNpdu {
        source: 7,
        destination: 1,
        npdu,
        request_id,
    }
}

fn bip_egress<'a, 'b>(
    sink: &'a mut Sink,
    scratch: &'b mut [u8],
    now: Instant,
) -> BipEgress<'a, 'b, Sink> {
    BipEgress::new(
        sink,
        scratch,
        PortId(3),
        Ipv4SocketAddress {
            ip: [192, 0, 2, 255],
            port: 47_808,
        },
        now,
    )
}

#[test]
fn real_bip_success_and_admission_failures_are_observed_once() {
    let npdu = routed_npdu(40, 300, &[192, 0, 2, 44, 0xba, 0xc1]);
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let mut scratch = [0; 512];
    assert!(matches!(
        forward_mstp_action_to_bip_observed(
            &mut router,
            &mut diagnostics,
            reverse_action(&npdu, Some(RequestId::new(9))),
            net(100),
            bip_caps(3, 300),
            &mut scratch,
            &mut bip,
            &mut bindings,
            1,
        ),
        Ok(ForwardOutcome::Forwarded { .. })
    ));
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.accepted, 1);
    assert_eq!(sink.calls, 1);

    // Sink-reported Full is surfaced through the concrete B/IP adapter.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::Full);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let outcome = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, None),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert_eq!(outcome, ForwardOutcome::QueueFull);
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.full, 1);

    // B/IP output storage is bounded independently of the router NPDU cap.
    let oversized = routed_npdu(600, 300, &[192, 0, 2, 44, 0xba, 0xc1]);
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 512];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let mut route_scratch = [0; 1600];
    let outcome = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&oversized, None),
        net(100),
        bip_caps(3, 300),
        &mut route_scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert!(matches!(outcome, ForwardOutcome::Oversize { .. }));
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.oversize, 1);

    // A regressed B/IP adapter timestamp is observable as Expired.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::from_micros(2));
    bip.set_now(Instant::from_micros(1));
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let outcome = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, None),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert_eq!(outcome, ForwardOutcome::Expired);
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.expired, 1);

    // Sink-reported Invalid remains a terminal malformed adapter result.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::Invalid);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let outcome = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, None),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert_eq!(outcome, ForwardOutcome::EgressRejected);
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.malformed, 1);
}

#[test]
fn reverse_binding_errors_are_observed_without_queue_attempt() {
    let npdu = routed_npdu(40, 300, &[192, 0, 2, 44, 0xba, 0xc1]);
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    assert_eq!(bindings.bind(RequestId::new(1), 1), BindingAdmission::Bound);
    let mut scratch = [0; 512];
    let result = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, Some(RequestId::new(2))),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    );
    assert_eq!(
        result,
        Err(ObservedCompositionError::Composition(
            CompositionError::Binding(BindingAdmission::Full),
        ))
    );
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.full, 1);
    assert_eq!(sink.calls, 0);

    // A duplicate request generation is an invalid binding and is observed
    // without entering the B/IP egress.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    assert_eq!(bindings.bind(RequestId::new(1), 1), BindingAdmission::Bound);
    let result = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, Some(RequestId::new(1))),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    );
    assert_eq!(
        result,
        Err(ObservedCompositionError::Composition(
            CompositionError::Binding(BindingAdmission::Invalid),
        ))
    );
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.malformed, 1);
    assert_eq!(sink.calls, 0);

    // A caller-time regression is a distinct expired binding result.
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 2);
    let result = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        reverse_action(&npdu, Some(RequestId::new(2))),
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    );
    assert_eq!(
        result,
        Err(ObservedCompositionError::Composition(
            CompositionError::Binding(BindingAdmission::Expired),
        ))
    );
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.expired, 1);
    assert_eq!(sink.calls, 0);
}

#[test]
fn reverse_invalid_action_is_observed_before_queue_attempt() {
    let mut router = RouterCore::<2, 2>::new();
    let mut diagnostics = new_diagnostics();
    let mut sink = Sink::new(AdmissionOutcome::AcceptedOwned);
    let mut bip_scratch = [0; 1600];
    let mut bip = bip_egress(&mut sink, &mut bip_scratch, Instant::ZERO);
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let mut scratch = [0; 512];
    let result = forward_mstp_action_to_bip_observed(
        &mut router,
        &mut diagnostics,
        Action::Idle,
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    );
    assert_eq!(
        result,
        Err(ObservedCompositionError::Composition(
            CompositionError::InvalidAction,
        ))
    );
    assert_eq!(diagnostics.port(PortId(3)).unwrap().1.malformed, 1);
    assert_eq!(sink.calls, 0);
}

#[test]
fn adapter_seam_keeps_core_broadcast_and_control_observers_public() {
    // No adapter-side firmware loop is introduced: callers use the typed
    // core methods directly for these independent boundaries.
    let mut diagnostics = new_diagnostics();
    diagnostics
        .observe_broadcast_error(PortId(2), bacnet_router_core::BroadcastError::EmptyNpdu)
        .unwrap();
    assert_eq!(diagnostics.port(PortId(2)).unwrap().1.malformed, 1);
}
