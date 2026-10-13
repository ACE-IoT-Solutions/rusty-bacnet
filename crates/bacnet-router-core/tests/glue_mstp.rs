//! The MS/TP glue (feature `mstp`): a router's MS/TP port over
//! `bacnet-mstp-core`'s master core.

#![cfg(feature = "mstp")]

mod fixture;
mod glue;

use bacnet_mstp_codec::{Frame as MstpFrame, FrameType, BROADCAST_MAC};
use bacnet_mstp_core::{
    Action, CoreError, Instant, NetworkPriority, QueueStorage, ReplyDecision, RequestId,
};
use bacnet_router_core::mstp::{group, MstpPort, NotExtended, BROADCAST};
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::router::{Group, Limits, Options, Received, SendRefusal};
use bacnet_router_core::{Mac, PortId};

use fixture::*;
use glue::*;

/// The MS/TP port: port 1, on network 2. Port 0 is a B/IP-like port on
/// network 1.
const MSTP_PORT: MstpPort = MstpPort::new(PortId(1));

/// The router, its MS/TP port configured from the core's configuration.
fn router() -> TestRouter {
    let mstp = MstpPort::config(&config(true), 2).unwrap();
    router_with(&[port_on(1, OWN_0), mstp], Options::HOSTED)
}

/// An APDU for `dnet`/`dadr` at network priority `priority`.
fn apdu_at(dnet: u16, dadr: &[u8], apdu: &[u8], priority: u8) -> Vec<u8> {
    let mut npdu = Npdu::apdu(apdu);
    npdu.destination = Some(addr(dnet, dadr));
    npdu.priority = priority;
    bytes(&npdu)
}

/// What the core delivers next: its source, destination and request, and
/// the NPDU.
fn delivered<const U: usize>(core: &mut Core<'_, U>) -> (u8, u8, Option<RequestId>, Vec<u8>) {
    loop {
        match core.poll_action() {
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                request_id,
            } => return (source, destination, request_id, npdu.to_vec()),
            Action::SetDirection(_) => {}
            other => panic!("expected a delivery, got {other:?}"),
        }
    }
}

#[test]
fn config_needs_extended_frames() {
    assert_eq!(MstpPort::config(&config(false), 2), Err(NotExtended));
    let port = MstpPort::config(&config(true), 2).unwrap();
    assert_eq!(port.network, 2);
    assert_eq!(port.mac, Mac::from_array([STATION]));
    assert_eq!(port.max_npdu, Limits::both(1497));
}

#[test]
fn the_broadcast_station_is_the_only_group() {
    assert_eq!(BROADCAST, BROADCAST_MAC);
    assert_eq!(group(&[BROADCAST]), Group::Broadcast);
    assert_eq!(group(&[5]), Group::No);
    assert_eq!(group(&[]), Group::No);
}

/// A broadcast on the MS/TP port reaches the router as a link broadcast
/// from its station, and a directed frame as a unicast.
#[test]
fn deliveries_become_frames() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let mut core = bus.core(true);
    let mut router = router();
    let mut host = Recorder::default();

    let global = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    let frame = MstpFrame::new(
        FrameType::BACnetDataNotExpectingReply,
        BROADCAST,
        7,
        &global,
    );
    core.on_validated_frame(frame, now).unwrap();
    let (source, destination, request, npdu) = delivered(&mut core);
    let arrived = MSTP_PORT.arrived(source, destination, &npdu);
    assert_eq!(
        (
            arrived.port,
            arrived.source.as_slice(),
            arrived.link_group,
            request
        ),
        (PortId(1), &[7][..], true, None)
    );
    assert!(matches!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Local(local) if local.global
    ));
    assert_eq!(drain(&mut router).len(), 1, "relayed to network 1");

    let directed = MSTP_PORT.arrived(7, STATION, &npdu);
    assert!(!directed.link_group);
}

/// A request for another network gets Reply Postponed at once; one the
/// router's own application answered keeps its answer (Clauses 6.5.4,
/// 9.5.6.9).
#[test]
fn relayed_requests_are_postponed() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let mut core = bus.core(true);
    let mut router = router();
    let mut host = Recorder::default();

    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(1, NODE_A));
    request.expecting_reply = true;
    let request = bytes(&request);
    let frame = MstpFrame::new(FrameType::BACnetDataExpectingReply, STATION, 7, &request);
    core.on_validated_frame(frame, now).unwrap();
    let (source, destination, id, npdu) = delivered(&mut core);
    assert!(id.is_some());
    let arrived = MSTP_PORT.arrived(source, destination, &npdu);
    assert_eq!(
        router.receive(arrived.frame(), now, &mut host),
        Received::Handled
    );
    let sent = drain(&mut router);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (0, Some(NODE_A)));

    MstpPort::release(&mut core, id, now).unwrap();
    assert_eq!(core.snapshot().request_id, None);
    let answer = next_frame(&mut core, now).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination),
        (FrameType::ReplyPostponed, 7)
    );

    // A request for the router's own application, which answers it.
    let mut local = Npdu::apdu(CONFIRMED);
    local.expecting_reply = true;
    let local = bytes(&local);
    let later = now + bacnet_router_core::Duration::from_millis(100);
    let frame = MstpFrame::new(FrameType::BACnetDataExpectingReply, STATION, 7, &local);
    core.on_validated_frame(frame, later).unwrap();
    let (source, destination, id, npdu) = delivered(&mut core);
    let arrived = MSTP_PORT.arrived(source, destination, &npdu);
    assert!(matches!(
        router.receive(arrived.frame(), later, &mut host),
        Received::Local(local) if local.expecting_reply
    ));
    let reply = [0x30, 0x05, 0x0C];
    core.complete_request(id.unwrap(), ReplyDecision::Data(&reply), later)
        .unwrap();
    MstpPort::release(&mut core, id, later).unwrap();
    MstpPort::release(&mut core, None, later).unwrap();
    let answer = next_frame(&mut core, later).unwrap();
    assert_eq!(
        (answer.frame_type, answer.destination, answer.data),
        (FrameType::BACnetDataNotExpectingReply, 7, reply.to_vec())
    );
    // A clock that ran backwards is the core's to report.
    let stale = RequestId::new(99);
    assert!(matches!(
        MstpPort::release(&mut core, Some(stale), Instant::ZERO),
        Err(CoreError::ClockRegression { .. })
    ));
}

/// What the router relays at life-safety priority leaves the MS/TP queues
/// before what it relayed earlier at normal priority.
#[test]
fn priority_reaches_the_queue() {
    let now = Instant::from_millis(1);
    let mut bus = Bus::<2>::new();
    let normal = apdu_at(2, &[5], CONFIRMED, 0);
    let urgent = apdu_at(2, &[6], CONFIRMED, 3);
    {
        let mut core = bus.core(true);
        let mut router = router();
        let mut host = Recorder::default();
        for npdu in [&normal, &urgent] {
            router.receive(frame_on(0, NODE_A, npdu), now, &mut host);
            let results = pump(&mut router, |send| MSTP_PORT.send(&mut core, send, now));
            assert_eq!(results, [(1, Ok(()))]);
        }
    }
    let first = bus.queues.dequeue(now, 1497).unwrap().unwrap();
    assert_eq!(
        (first.destination, first.priority),
        (6, NetworkPriority::LifeSafety)
    );
    let second = bus.queues.dequeue(now, 1497).unwrap().unwrap();
    assert_eq!(
        (second.destination, second.priority),
        (5, NetworkPriority::Normal)
    );
}

/// Each refusal reaches the router as its own, and is counted there once.
#[test]
fn refusals_map_to_the_routers() {
    let now = Instant::from_millis(1);
    let mut router = router();
    let mut host = Recorder::default();
    let mut route = |router: &mut TestRouter, npdu: &[u8], core: &mut Core<'_, 1>| {
        router.receive(frame_on(0, NODE_A, npdu), now, &mut host);
        pump(router, |send| MSTP_PORT.send(core, send, now))
    };

    // One unicast slot: the second waits for none.
    let mut bus = Bus::<1>::new();
    let mut core = bus.core(true);
    let fits = apdu_at(2, &[5], CONFIRMED, 0);
    assert_eq!(route(&mut router, &fits, &mut core), [(1, Ok(()))]);
    assert_eq!(
        route(&mut router, &fits, &mut core),
        [(1, Err(SendRefusal::Full))]
    );
    // Station 255 is no unicast destination, nor is a two-octet MAC.
    for dadr in [&[BROADCAST][..], &[1, 2]] {
        let npdu = apdu_at(2, dadr, CONFIRMED, 0);
        assert_eq!(
            route(&mut router, &npdu, &mut core),
            [(1, Err(SendRefusal::Other))]
        );
    }

    // A core without extended frames refuses an NPDU past 501 octets.
    let mut bus = Bus::<1>::new();
    let mut core = bus.core(false);
    let long = apdu_at(2, &[5], &[0x10; 510], 0);
    assert_eq!(
        route(&mut router, &long, &mut core),
        [(1, Err(SendRefusal::TooLong))]
    );
    // A send for another port.
    let other = MstpPort::new(PortId(0));
    router.receive(frame_on(0, NODE_A, &fits), now, &mut host);
    let results = pump(&mut router, |send| other.send(&mut core, send, now));
    assert_eq!(results, [(1, Err(SendRefusal::Other))]);

    let counts = router.counters();
    assert_eq!((counts.refused, counts.egress_too_long), (5, 1));
}

/// A frame port `port` received from `source`, by unicast.
fn frame_on<'f>(
    port: u8,
    source: &'f [u8],
    npdu: &'f [u8],
) -> bacnet_router_core::router::Frame<'f> {
    bacnet_router_core::router::Frame {
        port: PortId(port),
        source,
        link_group: false,
        npdu,
    }
}
