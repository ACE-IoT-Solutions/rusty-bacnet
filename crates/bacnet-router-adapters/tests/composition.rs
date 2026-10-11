#![allow(clippy::too_many_arguments)]

#[path = "../src/bip.rs"]
mod bip;
#[path = "../src/composition.rs"]
mod composition;
#[path = "../src/mstp.rs"]
mod mstp;

use bip::{BipDatagramSink, BipEgress};
use composition::{
    forward_bip_datagram_to_mstp, forward_mstp_action_to_bip, BindingAdmission, CompositionError,
    RequestBindings,
};
use mstp::MstpEgress;

use bacnet_mstp_codec::{decode_frame, Frame, FrameType};
use bacnet_mstp_core::{
    Action, HeaplessQueues, Instant, MasterCore, MstpCoreConfig, ReplyDecision,
};
use bacnet_router_core::{
    encode_forwarded_npdu, AdmissionOutcome, BipDecodeError, Egress, EgressDestination,
    Ipv4SocketAddress, LinkAddress, NetworkNumber, PortCapabilities, PortId, PortKind, RouterCore,
    UdpDatagramMeta,
};

type Queues = HeaplessQueues<1, 1>;

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

fn core<'rx>(rx: &'rx mut [u8]) -> MasterCore<'rx, Queues> {
    MasterCore::new(
        MstpCoreConfig {
            station: 1,
            max_master: 2,
            ..MstpCoreConfig::default()
        },
        Queues::new(),
        rx,
        Instant::ZERO,
    )
    .unwrap()
}

fn routed_npdu(total_len: usize, network: u16, destination: &[u8]) -> Vec<u8> {
    let mut npdu = vec![0x5a; total_len];
    npdu[0] = 1;
    npdu[1] = 0x23; // DNET present, life-safety priority
    npdu[2..4].copy_from_slice(&network.to_be_bytes());
    npdu[4] = destination.len() as u8;
    npdu[5..5 + destination.len()].copy_from_slice(destination);
    npdu[5 + destination.len()] = 0xff;
    npdu
}

fn metadata(peer: Ipv4SocketAddress, group: bool) -> UdpDatagramMeta {
    UdpDatagramMeta {
        peer,
        destination: Some(if group {
            [192, 0, 2, 255]
        } else {
            [192, 0, 2, 20]
        }),
        destination_is_group: group,
        truncated: false,
    }
}

#[derive(Clone, Copy)]
struct Sink {
    outcome: AdmissionOutcome,
    calls: usize,
    destination: Ipv4SocketAddress,
    frame: [u8; 1600],
    len: usize,
}

impl Sink {
    const fn accepting() -> Self {
        Self {
            outcome: AdmissionOutcome::AcceptedOwned,
            calls: 0,
            destination: Ipv4SocketAddress {
                ip: [0; 4],
                port: 0,
            },
            frame: [0; 1600],
            len: 0,
        }
    }
}

impl BipDatagramSink for Sink {
    fn try_send(
        &mut self,
        destination: Ipv4SocketAddress,
        datagram: &[u8],
        _now: Instant,
    ) -> AdmissionOutcome {
        self.calls += 1;
        self.destination = destination;
        if self.outcome == AdmissionOutcome::AcceptedOwned {
            self.frame[..datagram.len()].copy_from_slice(datagram);
            self.len = datagram.len();
        }
        self.outcome
    }
}

#[test]
fn strict_bip_failure_precedes_router_or_queue_mutation() {
    let mut router = RouterCore::<2, 2>::new();
    let mut rx = [0; 1024];
    let mut mstp_core = core(&mut rx);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::from_micros(1));
    let mut scratch = [0; 512];
    let error = forward_bip_datagram_to_mstp(
        &mut router,
        &[0x81, 0x0a],
        metadata(
            Ipv4SocketAddress {
                ip: [198, 51, 100, 2],
                port: 47_809,
            },
            false,
        ),
        Instant::from_micros(1),
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    )
    .unwrap_err();
    assert_eq!(
        error,
        CompositionError::BipDecode(BipDecodeError::HeaderTruncated)
    );
    assert_eq!(router.diagnostics().ingress_npdus, 0);
    assert_eq!(mstp.core_mut().counters().unicast.depth, 0);
}

#[test]
fn forwarded_origin_and_priority_survive_bip_to_mstp_composition() {
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(200), PortId(2)).unwrap();
    let origin = Ipv4SocketAddress {
        ip: [198, 51, 100, 10],
        port: 47_809,
    };
    let peer = Ipv4SocketAddress {
        ip: [203, 0, 113, 4],
        port: 47_809,
    };
    let npdu = routed_npdu(100, 200, &[5]);
    let mut datagram = [0; 1600];
    let datagram_len = encode_forwarded_npdu(&mut datagram, origin, &npdu).unwrap();
    let mut rx = [0; 1024];
    let mut mstp_core = core(&mut rx);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::from_micros(1));
    let mut scratch = [0; 512];
    let (context, outcome) = forward_bip_datagram_to_mstp(
        &mut router,
        &datagram[..datagram_len],
        metadata(peer, false),
        Instant::from_micros(1),
        net(100),
        mstp_caps(2, 200),
        &mut scratch,
        &mut mstp,
    )
    .unwrap();
    assert_eq!(context.ingress.immediate_peer, peer);
    assert_eq!(context.ingress.logical_source, origin);
    assert_eq!(
        outcome,
        bacnet_router_core::ForwardOutcome::Forwarded {
            final_npdu_len: 104
        }
    );
    assert_eq!(mstp.core_mut().counters().unicast.depth, 1);

    mstp.core_mut()
        .on_validated_frame(
            Frame::new(FrameType::Token, 1, 2, &[]),
            Instant::from_micros(2),
        )
        .unwrap();
    mstp.core_mut()
        .on_deadline(Instant::from_micros(50_000))
        .unwrap();
    let frame = loop {
        match mstp.core_mut().poll_action() {
            Action::Transmit { frame, .. } => break frame,
            Action::Idle | Action::SetDirection(_) => {}
            Action::DeliverNpdu { .. } => panic!("unexpected delivery"),
        }
    };
    let (frame, _) = decode_frame(frame).unwrap();
    assert_eq!(frame.header.destination, 5);
    assert_eq!(
        &frame.data[..11],
        &[0x01, 0x0b, 0x00, 0x64, 0x06, 198, 51, 100, 10, 0xba, 0xc1]
    );
    assert_eq!(frame.data[1] & 0x03, 3);
}

#[test]
fn mstp_delivery_becomes_bip_original_unicast_and_binds_reply_id() {
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let destination = Ipv4SocketAddress {
        ip: [192, 0, 2, 44],
        port: 47_809,
    };
    let npdu = routed_npdu(
        40,
        300,
        &[
            destination.ip[0],
            destination.ip[1],
            destination.ip[2],
            destination.ip[3],
            0xba,
            0xc1,
        ],
    );
    let action = Action::DeliverNpdu {
        source: 7,
        destination: 1,
        npdu: &npdu,
        request_id: Some(bacnet_mstp_core::RequestId::new(9)),
    };
    let mut sink = Sink::accepting();
    let mut bip_scratch = [0; 1600];
    let mut bip = BipEgress::new(
        &mut sink,
        &mut bip_scratch,
        PortId(3),
        Ipv4SocketAddress {
            ip: [192, 0, 2, 255],
            port: 47_808,
        },
        Instant::ZERO,
    );
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let mut scratch = [0; 512];
    let outcome = forward_mstp_action_to_bip(
        &mut router,
        action,
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert_eq!(
        outcome,
        bacnet_router_core::ForwardOutcome::Forwarded { final_npdu_len: 34 }
    );
    assert_eq!(bindings.len(), 1);
    drop(bip);
    assert_eq!(sink.calls, 1);
    assert_eq!(sink.destination, destination);
    let decoded = bip::decode_ingress(
        &sink.frame[..sink.len],
        metadata(
            Ipv4SocketAddress {
                ip: [192, 0, 2, 1],
                port: 47_808,
            },
            false,
        ),
        Instant::ZERO,
    )
    .unwrap();
    assert_eq!(
        decoded.ingress.function,
        bacnet_router_core::BvlcFunction::OriginalUnicastNpdu
    );
    assert_eq!(decoded.ingress.npdu[1] & 0x03, 3);
    assert_eq!(decoded.ingress.npdu[2..4], [0, 100]);
    assert_eq!(&decoded.ingress.npdu[4..6], &[1, 7]);
}

#[test]
fn mstp_broadcast_delivery_becomes_bip_original_broadcast() {
    let mut router = RouterCore::<2, 2>::new();
    router.routes_mut().add_direct(net(300), PortId(3)).unwrap();
    let broadcast = Ipv4SocketAddress {
        ip: [192, 0, 2, 255],
        port: 47_808,
    };
    let npdu = routed_npdu(20, 300, &[]);
    let action = Action::DeliverNpdu {
        source: 7,
        destination: 0xff,
        npdu: &npdu,
        request_id: None,
    };
    let mut sink = Sink::accepting();
    let mut bip_scratch = [0; 1600];
    let mut bip = BipEgress::new(
        &mut sink,
        &mut bip_scratch,
        PortId(3),
        broadcast,
        Instant::ZERO,
    );
    let mut bindings = RequestBindings::<1>::new(100, 0);
    let mut scratch = [0; 512];
    let outcome = forward_mstp_action_to_bip(
        &mut router,
        action,
        net(100),
        bip_caps(3, 300),
        &mut scratch,
        &mut bip,
        &mut bindings,
        1,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        bacnet_router_core::ForwardOutcome::Forwarded { .. }
    ));
    drop(bip);
    assert_eq!(sink.destination, broadcast);
    let decoded = bip::decode_ingress(
        &sink.frame[..sink.len],
        metadata(
            Ipv4SocketAddress {
                ip: [192, 0, 2, 1],
                port: 47_808,
            },
            true,
        ),
        Instant::ZERO,
    )
    .unwrap();
    assert_eq!(
        decoded.ingress.function,
        bacnet_router_core::BvlcFunction::OriginalBroadcastNpdu
    );
    assert_eq!(&decoded.ingress.npdu[2..4], &[0, 100]);
    assert_eq!(&decoded.ingress.npdu[4..6], &[1, 7]);
}

#[test]
fn binding_full_expired_invalid_and_timeout_are_explicit() {
    let id = bacnet_mstp_core::RequestId::new(4);
    let mut bindings = RequestBindings::<1>::new(10, 10);
    assert_eq!(bindings.bind(id, 10), BindingAdmission::Bound);
    assert_eq!(
        bindings.bind(bacnet_mstp_core::RequestId::new(5), 10),
        BindingAdmission::Full
    );
    assert_eq!(
        bindings.bind(bacnet_mstp_core::RequestId::new(6), 9),
        BindingAdmission::Expired
    );
    assert_eq!(bindings.bind(id, 11), BindingAdmission::Invalid);
    assert!(bindings.unbind(id));
}

#[test]
fn binding_calls_core_on_reply_and_timeout_without_hidden_payload_storage() {
    let mut rx = [0; 1024];
    let mut mstp = core(&mut rx);
    mstp.on_validated_frame(
        Frame::new(FrameType::BACnetDataExpectingReply, 1, 7, &[0, 1]),
        Instant::from_micros(1),
    )
    .unwrap();
    let id = match mstp.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    let mut bindings = RequestBindings::<1>::new(100, 0);
    assert_eq!(bindings.bind(id, 1), BindingAdmission::Bound);
    bindings
        .complete(
            &mut mstp,
            id,
            ReplyDecision::Data(&[0, 1, 2]),
            Instant::from_micros(2),
        )
        .unwrap();
    assert!(bindings.is_empty());
    assert_eq!(mstp.snapshot().request_id, None);

    let mut rx = [0; 1024];
    let mut mstp = core(&mut rx);
    mstp.on_validated_frame(
        Frame::new(FrameType::BACnetDataExpectingReply, 1, 7, &[0, 1]),
        Instant::from_micros(1),
    )
    .unwrap();
    let id = match mstp.poll_action() {
        Action::DeliverNpdu {
            request_id: Some(id),
            ..
        } => id,
        other => panic!("expected request delivery, got {other:?}"),
    };
    let mut bindings = RequestBindings::<1>::new(100, 0);
    assert_eq!(bindings.bind(id, 1), BindingAdmission::Bound);
    bindings
        .timeout(&mut mstp, id, Instant::from_micros(2))
        .unwrap();
    assert!(bindings.is_empty());
    assert_eq!(mstp.snapshot().request_id, None);
}

#[test]
fn standard_boundary_and_burst_backpressure_are_visible_at_mstp_queue() {
    let mut rx = [0; 1024];
    let mut mstp_core = core(&mut rx);
    let mut mstp = MstpEgress::new(&mut mstp_core, PortId(2), Instant::ZERO);
    let destination = EgressDestination::Unicast(LinkAddress::try_from_slice(&[5]).unwrap());
    let accepted = [0; 501];
    assert_eq!(
        mstp.try_admit(bacnet_router_core::EgressRequest {
            port: PortId(2),
            destination,
            class: bacnet_router_core::TrafficClass::Data,
            priority: 0,
            npdu: &accepted,
        }),
        AdmissionOutcome::AcceptedOwned
    );
    let oversized = [0; 502];
    assert_eq!(
        mstp.try_admit(bacnet_router_core::EgressRequest {
            port: PortId(2),
            destination,
            class: bacnet_router_core::TrafficClass::Data,
            priority: 0,
            npdu: &oversized,
        }),
        AdmissionOutcome::Oversize
    );
    assert_eq!(mstp.core_mut().counters().unicast.depth, 1);
    assert_eq!(
        mstp.try_admit(bacnet_router_core::EgressRequest {
            port: PortId(2),
            destination,
            class: bacnet_router_core::TrafficClass::Data,
            priority: 0,
            npdu: &[1],
        }),
        AdmissionOutcome::Full
    );
    assert_eq!(mstp.core_mut().counters().unicast.depth, 1);
}
