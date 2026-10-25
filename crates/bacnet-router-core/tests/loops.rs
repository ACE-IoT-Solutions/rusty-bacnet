//! Network messages relayed around a loop of routers (#1810).
//!
//! Three routers join networks 1, 2 and 3 in a ring, which the single path
//! of Clauses 4.2 and 6.2.3 rules out but a misconfigured site can have.
//! An I-Am-Router, Router-Busy or Router-Available has no DNET, so no hop
//! count, and each router passes it on whether or not its table changed
//! (Clauses 6.6.3.3, 6.6.3.6, 6.6.3.7): without duplicate suppression one
//! announcement circulates for good. With [`Options::duplicates`] each
//! router takes the first copy and drops the ones that come back.

mod fixture;

use std::collections::VecDeque;

use bacnet_router_core::Instant;
use bacnet_router_core::npdu::NetworkMessageType;
use bacnet_router_core::router::{Discard, Duplicates, Options, Permissive};
use bacnet_router_core::table::Reachability;
use fixture::{Outcome, TestRouter, deliver_broadcast, message, networks, port_on, router_with};

/// Router X, outside the ring, on network 1.
const X: &[u8] = &[0x99];

/// The ring: each router's two ports, as network and MAC.
const RING: [[(u16, u8); 2]; 3] = [
    [(1, 0x11), (2, 0x12)],
    [(2, 0x21), (3, 0x22)],
    [(3, 0x31), (1, 0x32)],
];

/// Deliveries after which a run counts as unbounded.
const LIMIT: usize = 10_000;

/// What the Busy and Available pair takes before every copy is dropped.
const PAIR_DELIVERIES: usize = 44;

fn ring(options: Options) -> Vec<TestRouter> {
    RING.iter()
        .map(|ports| {
            let ports = ports.map(|(network, mac)| port_on(network, &[mac]));
            router_with(&ports, options)
        })
        .collect()
}

/// What a run around the ring did.
#[derive(Debug)]
struct Run {
    /// Frames delivered to a router port.
    deliveries: usize,
    /// Of them, the ones dropped as repeats.
    duplicates: usize,
}

/// A broadcast on a network: the network, the router and port that sent
/// it (`None` for X), and the NPDU.
type OnWire = (u16, Option<(usize, usize)>, Vec<u8>);

/// X broadcasts `frame` on network 1; every broadcast a router then sends
/// reaches the other router ports on that network, until none is left or
/// [`LIMIT`] deliveries have been made. Each delivery takes a millisecond.
fn circulate(routers: &mut [TestRouter], frame: Vec<u8>) -> Run {
    circulate_all(routers, vec![frame])
}

/// [`circulate`] for several frames X sends one after another.
fn circulate_all(routers: &mut [TestRouter], frames: Vec<Vec<u8>>) -> Run {
    let mut wire: VecDeque<OnWire> = VecDeque::new();
    wire.extend(frames.into_iter().map(|frame| (1, None, frame)));
    let mut run = Run {
        deliveries: 0,
        duplicates: 0,
    };
    while let Some((network, from, frame)) = wire.pop_front() {
        let source = from.map_or(X.to_vec(), |(router, port)| vec![RING[router][port].1]);
        for (index, router) in routers.iter_mut().enumerate() {
            for (port, &(on, _)) in RING[index].iter().enumerate() {
                if on != network || from == Some((index, port)) {
                    continue;
                }
                if run.deliveries == LIMIT {
                    return run;
                }
                run.deliveries += 1;
                let now = Instant::from_millis(run.deliveries as u64);
                let (outcome, sent) =
                    deliver_broadcast(router, &mut Permissive, port as u8, &source, &frame, now);
                if outcome == Outcome::Discarded(Discard::Duplicate) {
                    run.duplicates += 1;
                }
                for send in sent {
                    assert!(send.is_broadcast());
                    let out = usize::from(send.port);
                    wire.push_back((RING[index][out].0, Some((index, out)), send.npdu));
                }
            }
        }
    }
    run
}

/// The messages that are passed on with no origin.
fn relayed() -> [(&'static str, Vec<u8>); 3] {
    [
        ("I-Am-Router", i_am(&[9])),
        ("Router-Busy", busy(&[9])),
        ("Router-Available", available(&[])),
    ]
}

fn i_am(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(list))
}

fn busy(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(list))
}

fn available(list: &[u16]) -> Vec<u8> {
    message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(list),
    )
}

#[test]
fn without_suppression_a_relayed_message_circulates_for_good() {
    for (name, frame) in relayed() {
        let mut routers = ring(Options::HOSTED);
        let run = circulate(&mut routers, frame);
        assert_eq!(run.deliveries, LIMIT, "{name}: still going");
    }
}

#[test]
fn with_suppression_each_router_takes_the_first_copy_only() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    // An I-Am-Router, kept by its data alone: X's frame reaches the two
    // routers on network 1, each relays it once, and the two copies that
    // come back round are dropped. A Router-Busy or Router-Available,
    // kept by its sender too: each router takes it once from each of its
    // two neighbours, so it goes round once each way before the third
    // copy from one neighbour is dropped.
    for ((name, frame), deliveries) in relayed().into_iter().zip([5, 10, 10]) {
        let mut routers = ring(options);
        let run = circulate(&mut routers, frame);
        assert_eq!(run.deliveries, deliveries, "{name}: {run:?}");
        assert_eq!(run.duplicates, 2, "{name}: {run:?}");
        let dropped: u32 = routers
            .iter()
            .map(|router| router.counters().duplicate_drops)
            .sum();
        assert_eq!(dropped, 2, "{name}");
    }
}

/// The looped copies are neither applied nor passed on: network 9 stays
/// where X's own announcement put it, on each router's port to network 1
/// for the two routers on it, and through network 2 for the third.
#[test]
fn a_looped_announcement_moves_no_route() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut routers = ring(options);
    let (_, frame) = relayed().into_iter().next().unwrap();
    let _ = circulate(&mut routers, frame);
    let ports: Vec<u8> = routers
        .iter()
        .map(|router| router.table().lookup(9).expect("learned").port().0)
        .collect();
    // R1's port 0 and R3's port 1 are on network 1; R2 heard it first
    // from R1, on its port to network 2.
    assert_eq!(ports, [0, 0, 1]);
    for router in &routers {
        let state = router
            .table()
            .effective_reachability(9, Instant::from_secs(1));
        assert_eq!(state, Some(Reachability::Reachable));
    }
}

/// One router, off the ring: the record keys a relayed message by its type
/// and data, forgets it after the time to live, and with suppression off
/// passes every copy on, as before.
#[test]
fn relayed_messages_are_keyed_by_type_and_data() {
    let on = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let busy = message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(&[9]));
    let available = message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(&[9]),
    );
    let i_am_9 = message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(&[9]));
    let i_am_8 = message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &networks(&[8]));
    let mut router = router_with(&fixture::two_ports(), on);
    let mut relayed = |frame: &[u8], millis: u64| {
        let now = Instant::from_millis(millis);
        let (outcome, sent) = deliver_broadcast(&mut router, &mut Permissive, 1, X, frame, now);
        (outcome != Outcome::Discarded(Discard::Duplicate)).then_some(sent.len())
    };
    // Same data, another type; another list; then the first again.
    assert_eq!(relayed(&busy, 0), Some(1));
    assert_eq!(relayed(&available, 1), Some(1), "another type");
    assert_eq!(relayed(&i_am_9, 2), Some(1));
    assert_eq!(relayed(&i_am_8, 3), Some(1), "another list");
    assert_eq!(relayed(&i_am_9, 999), None, "within the time to live");
    assert_eq!(relayed(&i_am_9, 1_002), Some(1), "after it");

    let mut router = router_with(&fixture::two_ports(), Options::HOSTED);
    for _ in 0..3 {
        let (outcome, sent) =
            deliver_broadcast(&mut router, &mut Permissive, 1, X, &i_am_9, Instant::ZERO);
        assert_eq!(
            (outcome, sent.len()),
            (Outcome::Handled, 1),
            "off by default"
        );
    }
}

/// A Router-Busy then a Router-Available from X: each accepted message
/// forgets its opposite from the same sender, so without a cap on those
/// resets the pair would pass round the ring for good, each copy the
/// opposite of the one before it on every link. The cap,
/// [`Duplicates::RESET_LIMIT`] resets per sender within one time to live,
/// stops it.
#[test]
fn an_alternating_busy_and_available_pair_is_bounded() {
    let options = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut routers = ring(options);
    let run = circulate_all(&mut routers, vec![busy(&[9]), available(&[9])]);
    assert!(run.deliveries < LIMIT, "{run:?}");
    assert_eq!(run.deliveries, PAIR_DELIVERIES, "{run:?}");
}
