//! Rewriting for the next hop (Clause 6.5.4; router/forwarding.rs), the
//! global fan-out, and R1's egress limits, which are off today.

mod fixture;

use bacnet_router_core::message::{code, reason};
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::router::{Cause, DropReason, Event, Fixes, Kind, Limits, Options};
use bacnet_router_core::PortId;
use fixture::{
    addr, apdu_to, at, bytes, deliver, learn_9_via_r7, port_on, router, router_with, sent_reject,
    two_ports, Outcome, Recorder, CONFIRMED, NODE_A, OWN_0, OWN_1, R7, UNCONFIRMED,
};

/// Clause 6.5.4: SNET and SADR are added from the arrival network and the
/// link source when missing, and kept when present.
#[test]
fn source_is_added_once_and_kept_after() {
    let mut router = router();
    let mut host = Recorder::default();
    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(sent[0].parsed().source, Some(addr(1, NODE_A)));

    let mut relayed = Npdu::apdu(CONFIRMED);
    relayed.destination = Some(addr(2, &[5]));
    relayed.source = Some(addr(7, &[1, 2, 3]));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&relayed), at(1));
    assert_eq!(sent[0].parsed().source, Some(addr(7, &[1, 2, 3])));
}

/// Clause 6.5.4: onto a directly attached network the DNET, DADR and hop
/// count go, and the NPDU goes to the DADR, or as a broadcast for DLEN 0.
/// Priority, expecting-reply and the APDU travel unchanged.
#[test]
fn direct_delivery_strips_destination() {
    let mut router = router();
    let mut host = Recorder::default();
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(2, &[5]));
    request.hop_count = 9;
    request.priority = 3;
    request.expecting_reply = true;
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(&[5][..])));
    assert_eq!(
        (sent[0].kind, sent[0].cause, sent[0].priority),
        (Kind::Forward, Cause::Ingress, 3)
    );
    let expected = Npdu {
        destination: None,
        source: Some(addr(1, NODE_A)),
        ..request
    };
    assert_eq!(sent[0].npdu, bytes(&expected));

    let (_, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[], UNCONFIRMED),
        at(1),
    );
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (1, true));
    assert_eq!(sent[0].parsed().destination, None);
}

/// Clause 6.5.4 and 6.2.3: toward another router the DNET and DADR stay,
/// the NPDU goes to that router, and the hop count drops by one.
#[test]
fn next_router_keeps_destination_and_counts_down() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut request = Npdu::apdu(CONFIRMED);
    request.destination = Some(addr(9, &[5]));
    request.hop_count = 17;
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
    assert_eq!(sent[0].to.as_deref(), Some(R7));
    let parsed = sent[0].parsed();
    assert_eq!(parsed.destination, Some(addr(9, &[5])));
    assert_eq!(parsed.hop_count, 16);
}

/// A hop count of 0 ends the relay; today one of 1 still leaves, with 0,
/// as the hosted router sends it (router/forwarding.rs:28-31, 51; fix R6
/// in #1679 changes that later).
#[test]
fn hop_count_zero_is_not_relayed_and_one_leaves_with_zero() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let hops = |count: u8| {
        let mut request = Npdu::apdu(CONFIRMED);
        request.destination = Some(addr(9, &[5]));
        request.hop_count = count;
        bytes(&request)
    };
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &hops(0), at(1));
    assert_eq!(outcome, Outcome::Dropped(DropReason::HopCount));
    assert!(sent.is_empty(), "no reject either");
    assert_eq!(router.counters().hop_count_drops, 1);
    assert_eq!(host.events, [Event::HopCountSpent { network: 9 }]);

    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &hops(1), at(1));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].parsed().hop_count, 0);
}

/// Clauses 6.3.2 and 6.5.4: a global broadcast goes to every attached
/// network but the one it came from, keeping DNET 0xFFFF and DLEN 0, and is
/// delivered locally too. With its hop count spent it is only delivered.
#[test]
fn global_broadcast_fans_out_and_delivers() {
    let ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        &[0x20],
        &apdu_to(0xFFFF, &[], UNCONFIRMED),
        at(1),
    );
    assert!(matches!(
        outcome,
        Outcome::Local {
            group: true,
            global: true,
            ..
        }
    ));
    let ports: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(ports, [0, 2]);
    for sent in &sent {
        assert!(sent.is_broadcast());
        assert_eq!(sent.kind, Kind::Global);
        let parsed = sent.parsed();
        assert_eq!(parsed.destination, Some(addr(0xFFFF, &[])));
        assert_eq!(parsed.source, Some(addr(2, &[0x20])));
        assert_eq!(parsed.hop_count, 254);
    }

    // With 1 left it still goes on, with 0, as today (fix R6 later).
    let mut last = Npdu::apdu(UNCONFIRMED);
    last.destination = Some(addr(0xFFFF, &[]));
    last.hop_count = 1;
    let (_, sent) = deliver(&mut router, &mut host, 1, &[0x20], &bytes(&last), at(1));
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|sent| sent.parsed().hop_count == 0));

    // Spent: delivered, not relayed, and the host hears of it, as the
    // hosted router logs it (router/forwarding.rs:117-120).
    let mut spent = last;
    spent.hop_count = 0;
    let (outcome, sent) = deliver(&mut router, &mut host, 1, &[0x20], &bytes(&spent), at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    assert!(sent.is_empty());
    assert_eq!(router.counters().hop_count_drops, 1);
    assert_eq!(host.events, [Event::HopCountSpent { network: 0xFFFF }]);
}

/// Clauses 6.2.5 and 6.5.4: a network message for another network, a
/// proprietary one included, is routed like an APDU, with its type and
/// vendor ID kept, and the router never takes it itself.
#[test]
fn proprietary_messages_route_opaquely() {
    let mut router = router();
    let mut host = Recorder::default();
    learn_9_via_r7(&mut router, at(0));
    let mut message = Npdu::message(0x80, &[0xDE, 0xAD]);
    message.vendor_id = Some(260);
    message.destination = Some(addr(9, &[5]));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&message), at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent[0].to.as_deref(), Some(R7));
    let parsed = sent[0].parsed();
    assert_eq!(
        (parsed.message_type, parsed.vendor_id),
        (Some(0x80), Some(260))
    );
    assert_eq!(parsed.payload, [0xDE, 0xAD]);

    // An I-Am-Router for another network is routed too, not learned from.
    let mut i_am = Npdu::message(code::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
    i_am.destination = Some(addr(9, &[5]));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&i_am), at(1));
    assert_eq!((sent.len(), sent[0].kind), (1, Kind::Forward));
    assert!(router.table().lookup(44).is_none());
    assert!(host.asked.is_empty());
}

/// What the writer can't write goes nowhere and is counted, as an encode
/// failure is logged and dropped today: an SNET of 0 from a port numbered
/// 0, an empty SADR from an empty link source, and a rewrite longer than
/// the scratch.
#[test]
fn unwritable_rewrites_are_dropped_and_counted() {
    let ports = [port_on(0, OWN_0), port_on(2, OWN_1)];
    let mut router = router_with(&ports, Options::HOSTED);
    let mut host = Recorder::default();
    let unwritable = Outcome::Dropped(DropReason::Unwritable);
    let (outcome, _) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, unwritable);

    let mut router = fixture::router();
    let (outcome, _) = deliver(
        &mut router,
        &mut host,
        0,
        &[],
        &apdu_to(2, &[5], CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, unwritable);
    // 1,497 octets in, 1,501 out: the source fields (3 + 6 octets) come
    // in, the destination fields (3 + 1) and the hop count go.
    let mut long = Npdu::apdu(&[0u8; 1497 - 7]);
    long.destination = Some(addr(2, &[5]));
    let frame = bytes(&long);
    assert_eq!(frame.len(), 1497);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(outcome, unwritable);
    assert!(sent.is_empty());
    assert_eq!(router.counters().write_failures, 2);
}

/// R1 (#1665), off today: with it on, an NPDU too long for its egress port
/// draws reason 4 naming its DNET, and a fan-out leaves that port out,
/// counted, with no reject (decision D21 on #1679).
#[test]
fn egress_limits_with_r1_on() {
    let options = Options {
        fixes: Fixes {
            egress_limits: true,
            ..Fixes::NONE
        },
        ..Options::HOSTED
    };
    let mut ports = [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, &[0x33])];
    ports[1].max_npdu = Limits::both(501);
    let mut router = router_with(&ports, options);
    let mut host = Recorder::default();

    // 498 octets in grow by 4 to 502, one past port 1's limit.
    let mut long = Npdu::apdu(&[0u8; 498 - 7]);
    long.destination = Some(addr(2, &[5]));
    let frame = bytes(&long);
    assert_eq!(frame.len(), 498);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to.as_deref(), Some(NODE_A));
    assert_eq!(sent_reject(&sent[0]), (reason::MESSAGE_TOO_LONG, 2, None));
    // One octet shorter fits exactly.
    let mut fits = Npdu::apdu(&[0u8; 497 - 7]);
    fits.destination = Some(addr(2, &[5]));
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&fits), at(1));
    assert_eq!((sent[0].port, sent[0].npdu.len()), (1, 501));

    // A global broadcast too long for port 1 still reaches port 2.
    let mut global = Npdu::apdu(&[0x10; 600]);
    global.destination = Some(addr(0xFFFF, &[]));
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&global), at(1));
    assert!(matches!(outcome, Outcome::Local { global: true, .. }));
    let ports: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(ports, [2]);
    assert_eq!(router.counters().egress_too_long, 2);
    assert_eq!(router.counters().rejects, 1);

    // Off, the same NPDU goes out whatever the limit says.
    let mut router = router_with(&ports_with_limit(), Options::HOSTED);
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &frame, at(1));
    assert_eq!((sent[0].port, sent[0].npdu.len()), (1, 502));
}

/// The standard ports, with port 1 limited to 501 octets.
fn ports_with_limit() -> [bacnet_router_core::router::PortConfig; 2] {
    let mut ports = two_ports();
    ports[1].max_npdu = Limits::both(501);
    ports
}

/// The NPDU priority reaches every send, for a port that queues by it.
#[test]
fn sends_carry_the_npdu_priority() {
    let mut router = router();
    let mut host = Recorder::default();
    for priority in 0..=3 {
        let mut request = Npdu::apdu(CONFIRMED);
        request.destination = Some(addr(2, &[5]));
        request.priority = priority;
        let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &bytes(&request), at(1));
        assert_eq!(sent[0].priority, priority);
    }
    assert_eq!(router.table().lookup(2).unwrap().port, PortId(1));
}
