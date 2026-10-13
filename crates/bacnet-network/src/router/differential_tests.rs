//! The cutover's differential test (#1679): random runs of frames, time and
//! full queues against the legacy copy of the hosted router and the hosted
//! router on the engine, side by side on one paused clock, comparing after
//! every step every send (port, destination, octets, data attributes and
//! kind, in order), every local delivery and control, every count, what
//! the authorizer was asked, the tables and the searches
//! ([`Pair::check`]).
//!
//! Left out, as documented changes with their own tests in
//! `cutover_tests.rs`: an I-Could-Be-Router for a new network at the route
//! cap (stored before, refused now; [`Pair::step`] skips it and counts it),
//! router time on Tokio time (both run on it here), and the authorizer
//! running under the engine's lock (one thread here). The runs stay within
//! the engine's bounds the legacy router lacked: at most 255 ports, 1,024
//! hold-down records and NPDUs that fit its 65,556-octet scratch.
//!
//! Each case is generated from seeds by a small PRNG, so a failure prints
//! its steps.

use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_types::enums::NetworkPriority;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use proptest::prelude::*;

use super::differential_support::{Arrival, Gate, Groups, Pair, PortSpec, Scenario, Seed, Step};

/// A xorshift PRNG: the cases come from proptest's seeds.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True `num` times in `den`.
    fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
}

const BIP_OWN: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const BIP_NODES: [[u8; 6]; 2] = [[10, 0, 0, 9, 0xBA, 0xC0], [10, 0, 0, 7, 0xBA, 0xC0]];
const BIP_GROUPS: [[u8; 6]; 3] = [
    [255, 255, 255, 255, 0xBA, 0xC0],
    [224, 0, 0, 5, 0xBA, 0xC0],
    [255, 255, 255, 255, 0xBA, 0xC1],
];
const MSTP_OWN: [u8; 1] = [0x01];
const MSTP_NODES: [[u8; 1]; 2] = [[0x05], [0x07]];
const IP6_OWN: [u8; 3] = [0, 0, 1];
const IP6_NODES: [[u8; 3]; 2] = [[0, 0, 9], [0, 0, 7]];
/// A network beyond another router, unknown to every table.
const REMOTE: u16 = 7000;
/// Learned networks the seeds use.
const LEARNED: [u16; 6] = [4000, 4001, 4002, 4003, 4004, 4005];
/// Networks no table has.
const UNKNOWN: [u16; 2] = [5000, 5001];

fn bip(network: u16, groups: Groups) -> PortSpec {
    PortSpec {
        network,
        mac: BIP_OWN.to_vec(),
        groups,
    }
}

fn mstp(network: u16) -> PortSpec {
    PortSpec {
        network,
        mac: MSTP_OWN.to_vec(),
        groups: Groups::Mstp,
    }
}

fn ip6(network: u16) -> PortSpec {
    PortSpec {
        network,
        mac: IP6_OWN.to_vec(),
        groups: Groups::None,
    }
}

/// The nodes on a port, by the port's MAC width.
fn nodes(port: &PortSpec) -> Vec<Vec<u8>> {
    match port.mac.len() {
        6 => BIP_NODES.iter().map(|mac| mac.to_vec()).collect(),
        1 => MSTP_NODES.iter().map(|mac| mac.to_vec()).collect(),
        _ => IP6_NODES.iter().map(|mac| mac.to_vec()).collect(),
    }
}

/// A scenario: one of four port shapes, with seeds, holds, a gate and
/// queue sizes.
fn scenario(rng: &mut Rng) -> Scenario {
    let shape = rng.below(4);
    let ports = match shape {
        0 | 2 => vec![bip(1000, Groups::Bip), mstp(2000)],
        1 => vec![bip(1000, Groups::Bip), mstp(2000), ip6(3000)],
        // A port numbered 0, which gets no direct route.
        _ => vec![bip(1000, Groups::None), mstp(0)],
    };
    let mut seeds = Vec::new();
    let learned: Vec<u16> = if shape == 2 {
        // The full table: 2 direct routes and 254 learned ones.
        (10..264).collect()
    } else {
        LEARNED[..rng.below(LEARNED.len() + 1)].to_vec()
    };
    for network in learned {
        let port = rng.below(ports.len());
        let mark = match rng.below(6) {
            0 => Some(Some(1 + rng.below(40_000) as u64)),
            1 => Some(None),
            _ => None,
        };
        seeds.push(Seed {
            network,
            port,
            next_hop: rng.pick(&nodes(&ports[port])),
            mark,
        });
    }
    let holds = (0..rng.below(3))
        .filter_map(|_| {
            let seed = seeds.get(rng.below(seeds.len().max(1)))?;
            Some((
                seed.network,
                rng.below(ports.len()),
                rng.pick(&[0, 1, 2, 4]),
            ))
        })
        .collect();
    let gate = match rng.below(8) {
        0..=3 => Gate::Permissive,
        4 => Gate::Hardened,
        _ => Gate::Authorizer {
            mask: rng.next() as u32,
            hardened: rng.chance(1, 2),
        },
    };
    let capacities = ports
        .iter()
        .map(|_| rng.pick(&[1, 2, 4, 16, 256]))
        .collect();
    Scenario {
        ports,
        seeds,
        holds,
        hardened: rng.chance(1, 4),
        gate,
        capacities,
    }
}

/// Networks a frame may name: half the time one of a few, so claims,
/// marks and rejects meet the same routes again.
fn network(rng: &mut Rng, scenario: &Scenario) -> u16 {
    if rng.chance(1, 2) {
        let ports = scenario.ports.len();
        return match rng.below(ports + 3) {
            0 => LEARNED[0],
            1 => LEARNED[1],
            2 => UNKNOWN[0],
            port => scenario.ports[port - 3].network,
        };
    }
    match rng.below(20) {
        0..=5 => rng.pick(&scenario.ports).network,
        6..=10 => scenario
            .seeds
            .get(rng.below(scenario.seeds.len().max(1)) % 8)
            .map_or(LEARNED[0], |seed| seed.network),
        11..=13 => rng.pick(&UNKNOWN),
        14 => REMOTE,
        15..=16 => 0xFFFF,
        17 => 0,
        18 => 300 + rng.below(4) as u16,
        _ => rng.next() as u16,
    }
}

/// A MAC on some port, or a group, an empty one or one too long.
fn mac(rng: &mut Rng, scenario: &Scenario) -> Vec<u8> {
    let port = rng.pick(&scenario.ports);
    match rng.below(16) {
        0..=7 => rng.pick(&nodes(&port)),
        8..=9 => port.mac.clone(),
        10..=11 => rng.pick(&BIP_GROUPS).to_vec(),
        12 => vec![0xFF],
        13 => Vec::new(),
        14 => vec![0xAB; 18],
        _ => vec![0xAB; 19],
    }
}

fn list(rng: &mut Rng, scenario: &Scenario, max: usize) -> Vec<u8> {
    (0..rng.below(max + 1))
        .flat_map(|_| network(rng, scenario).to_be_bytes())
        .collect()
}

/// A network message's data, mostly in its type's layout.
fn message_data(rng: &mut Rng, scenario: &Scenario, kind: u8) -> Vec<u8> {
    let net = |rng: &mut Rng| network(rng, scenario).to_be_bytes();
    let mut data = match kind {
        0x00 => {
            if rng.chance(1, 3) {
                Vec::new()
            } else {
                net(rng).to_vec()
            }
        }
        0x01 => list(rng, scenario, 4),
        0x02 | 0x08 => [&net(rng)[..], &[rng.below(256) as u8]].concat(),
        0x03 => [&[rng.pick(&[0u8, 1, 2, 3, 4, 5, 6, 9])][..], &net(rng)].concat(),
        0x04 | 0x05 => list(rng, scenario, 3),
        0x06 | 0x07 => {
            if kind == 0x07 && rng.chance(1, 6) {
                return Vec::new();
            }
            let count = rng.below(4);
            let mut data = vec![count as u8];
            for _ in 0..count {
                data.extend(net(rng));
                data.push(rng.pick(&[0u8, 1, 2, 3, 4]));
                let info = rng.pick(&[0usize, 0, 0, 1, 2]);
                data.push(info as u8);
                data.extend(std::iter::repeat_n(0xAA, info));
            }
            data
        }
        0x09 => net(rng).to_vec(),
        0x12 => Vec::new(),
        0x13 => [&net(rng)[..], &[rng.pick(&[0u8, 1])]].concat(),
        _ => (0..rng.below(4)).map(|_| rng.next() as u8).collect(),
    };
    if rng.chance(1, 8) {
        data.pop();
    } else if rng.chance(1, 8) {
        data.push(0x5A);
    }
    data
}

/// An APDU: a PDU type octet and a little more, or nothing.
fn apdu(rng: &mut Rng) -> Vec<u8> {
    if rng.chance(1, 20) {
        return Vec::new();
    }
    let kind = rng.pick(&[0u8, 1, 1, 1, 2, 3, 5, 6, 7, 15]);
    let mut apdu = vec![kind << 4 | rng.below(16) as u8];
    apdu.extend((0..rng.below(4)).map(|_| rng.next() as u8));
    apdu
}

/// An NPDU's octets, any field allowed: a frame no encoder would make is
/// as likely as one it would.
fn npdu(rng: &mut Rng, scenario: &Scenario) -> Vec<u8> {
    let version = if rng.chance(1, 40) { 2 } else { 1 };
    let message = rng.chance(1, 2).then(|| {
        rng.pick(&[
            0x00u8, 0x00, 0x01, 0x01, 0x01, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x06, 0x07, 0x08,
            0x09, 0x0A, 0x11, 0x12, 0x13, 0x13, 0x14, 0x7F, 0x80, 0xFE,
        ])
    });
    let destination = (!rng.chance(1, 3)).then(|| {
        let dnet = network(rng, scenario);
        // A broadcast more often than a random DADR would make one.
        let dadr = if rng.chance(1, 3) {
            Vec::new()
        } else {
            mac(rng, scenario)
        };
        (dnet, dadr)
    });
    let source = rng.chance(1, 3).then(|| {
        let net = match rng.below(8) {
            0 => 0,
            1 => 0xFFFF,
            2..=4 => rng.pick(&scenario.ports).network,
            _ => REMOTE,
        };
        let mac = match rng.below(8) {
            0 => Vec::new(),
            1 => vec![0xCD; 19],
            2 => rng.pick(&scenario.ports).mac.clone(),
            _ => mac(rng, scenario),
        };
        (net, mac)
    });
    let mut control = rng.below(4) as u8;
    if message.is_some() {
        control |= 0x80;
    }
    if destination.is_some() {
        control |= 0x20;
    }
    if source.is_some() {
        control |= 0x08;
    }
    if rng.chance(1, 2) {
        control |= 0x04;
    }
    if rng.chance(1, 30) {
        control |= 0x50;
    }
    let mut out = vec![version, control];
    for (net, mac) in destination.iter().chain(source.iter()) {
        out.extend(net.to_be_bytes());
        out.push(mac.len() as u8);
        out.extend(mac);
    }
    if destination.is_some() {
        let any = rng.next() as u8;
        out.push(rng.pick(&[0u8, 1, 2, 255, 255, 255, any]));
    }
    match message {
        Some(kind) => {
            out.push(kind);
            if kind >= 0x80 {
                out.extend((rng.below(3) as u16 * 555).to_be_bytes());
            }
            out.extend(message_data(rng, scenario, kind));
        }
        None if destination
            .as_ref()
            .is_some_and(|(_, dadr)| dadr.is_empty())
            && rng.chance(2, 3) =>
        {
            out.extend([0x10, 0x08]);
        }
        None => out.extend(apdu(rng)),
    }
    if rng.chance(1, 30) {
        out.truncate(rng.below(out.len() + 1));
    }
    out
}

fn arrival(rng: &mut Rng, scenario: &Scenario) -> Arrival {
    let port = rng.below(scenario.ports.len());
    let source = match rng.below(32) {
        0 => Vec::new(),
        1 => vec![0xEE; 18],
        2 => vec![0xEE; 19],
        3 => scenario.ports[port].mac.clone(),
        4 => rng.pick(&BIP_GROUPS).to_vec(),
        // The router node, whose claims meet each other.
        5..=17 => nodes(&scenario.ports[port])[1].clone(),
        _ => rng.pick(&nodes(&scenario.ports[port])),
    };
    Arrival {
        port,
        source,
        link_group: rng.chance(1, 2),
        npdu: npdu(rng, scenario),
        attributes: rng.chance(1, 2),
        reply: rng.chance(1, 8),
    }
}

/// A well-formed claim of one of the few networks, from the router node
/// on some port: an I-Am-Router, or an Init-ACK. Claims that meet each
/// other move routes, warn of flapping and corroborate.
fn claim(rng: &mut Rng, scenario: &Scenario) -> Arrival {
    let port = rng.below(scenario.ports.len());
    let [high, low] = rng
        .pick(&[LEARNED[0], LEARNED[1], UNKNOWN[0]])
        .to_be_bytes();
    let npdu = if rng.chance(2, 3) {
        vec![1, 0x80, 0x01, high, low]
    } else {
        vec![1, 0x80, 0x07, 1, high, low, 0, 0]
    };
    Arrival {
        port,
        source: nodes(&scenario.ports[port])[1].clone(),
        link_group: true,
        npdu,
        attributes: false,
        reply: false,
    }
}

fn step(rng: &mut Rng, scenario: &Scenario) -> Step {
    match rng.below(100) {
        0..=11 => Step::Frame(claim(rng, scenario)),
        12..=71 => Step::Frame(arrival(rng, scenario)),
        72..=86 => Step::Advance(rng.pick(&[
            1, 500, 4_999, 5_000, 10_000, 29_999, 30_001, 60_000, 61_000, 300_001,
        ])),
        87..=93 => Step::Clog(rng.below(scenario.ports.len())),
        _ => Step::Unclog(rng.below(scenario.ports.len())),
    }
}

/// Run `steps` on `scenario`, comparing after each, then stop both.
fn run(scenario: &Scenario, steps: &[Step]) -> (usize, usize) {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(async {
            let mut pair = Pair::start(scenario).await;
            for step in steps {
                pair.step(step).await;
            }
            pair.stop().await;
            (pair.frames, pair.excluded)
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Random runs agree after every step.
    #[test]
    fn the_engine_router_matches_the_legacy_router(
        setup in any::<u64>(),
        seeds in prop::collection::vec(any::<u64>(), 1..96),
    ) {
        let mut rng = Rng::new(setup);
        let scenario = scenario(&mut rng);
        let steps: Vec<Step> = seeds
            .iter()
            .map(|&seed| step(&mut Rng::new(seed ^ setup), &scenario))
            .collect();
        let outcome = std::panic::catch_unwind(|| run(&scenario, &steps));
        prop_assert!(outcome.is_ok(), "scenario {scenario:#?}\nsteps {steps:#?}");
    }
}

// --- Fixed runs ---

fn frame(port: usize, source: &[u8], npdu: &Npdu) -> Step {
    let mut out = BytesMut::new();
    encode_npdu(&mut out, npdu).unwrap();
    Step::Frame(Arrival {
        port,
        source: source.to_vec(),
        link_group: false,
        npdu: out.to_vec(),
        attributes: true,
        reply: false,
    })
}

fn address(network: u16, mac: &[u8]) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    })
}

fn apdu_to(dnet: u16, dadr: &[u8], apdu: &[u8]) -> Npdu {
    Npdu {
        destination: address(dnet, dadr),
        hop_count: 255,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    }
}

fn message(kind: u8, data: &[u8]) -> Npdu {
    Npdu {
        is_network_message: true,
        message_type: Some(kind),
        payload: Bytes::copy_from_slice(data),
        ..Npdu::default()
    }
}

fn two_ports() -> Scenario {
    Scenario {
        ports: vec![bip(1000, Groups::Bip), mstp(2000)],
        seeds: Vec::new(),
        holds: Vec::new(),
        hardened: false,
        gate: Gate::Permissive,
        capacities: vec![16, 16],
    }
}

const NODE: &[u8] = &BIP_NODES[0];
const R7: &[u8] = &MSTP_NODES[1];
const CONFIRMED: &[u8] = &[0x00, 0x05, 0x01, 0x0C];
const UNCONFIRMED: &[u8] = &[0x10, 0x08];

/// Discovery, learning, forwarding and aging over six minutes.
#[test]
fn discovery_learning_and_aging_agree() {
    let mut steps = vec![
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
        Step::Advance(1_000),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
        frame(1, R7, &message(0x01, &[0, 9])),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
    ];
    for _ in 0..30 {
        steps.push(Step::Advance(10_000));
        steps.push(frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)));
    }
    steps.push(Step::Advance(60_000));
    steps.push(frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)));
    assert_eq!(run(&two_ports(), &steps), (35, 0));
}

/// Flow control, rejects with their hold-down, and the busy deadline.
#[test]
fn flow_control_and_rejects_agree() {
    let mut scenario = two_ports();
    scenario.seeds = vec![Seed {
        network: 9,
        port: 1,
        next_hop: R7.to_vec(),
        mark: None,
    }];
    let steps = vec![
        frame(1, R7, &message(0x04, &[0, 9])),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
        frame(1, R7, &message(0x05, &[])),
        frame(1, R7, &message(0x04, &[])),
        Step::Advance(29_999),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
        Step::Advance(1),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
        frame(1, R7, &message(0x03, &[1, 0, 9])),
        frame(1, R7, &message(0x03, &[2, 0, 9])),
        Step::Advance(30_000),
        frame(1, R7, &message(0x03, &[0, 0, 9])),
        frame(0, NODE, &apdu_to(9, &[3], CONFIRMED)),
    ];
    assert_eq!(run(&scenario, &steps), (10, 0));
}

/// A table query answered in several Acks, the rest of them dropped by a
/// queue that fills, and the update Ack.
#[test]
fn table_answers_on_a_full_queue_agree() {
    let mut scenario = two_ports();
    scenario.capacities = vec![1, 16];
    scenario.seeds = (10..210)
        .map(|network| Seed {
            network,
            port: 1,
            next_hop: R7.to_vec(),
            mark: None,
        })
        .collect();
    let steps = vec![
        Step::Clog(0),
        frame(0, NODE, &message(0x06, &[0])),
        Step::Unclog(0),
        frame(0, NODE, &message(0x06, &[0])),
        frame(0, NODE, &message(0x06, &[1, 0x0b, 0xb8, 2, 0])),
    ];
    assert_eq!(run(&scenario, &steps), (3, 0));
}

/// Group DADRs on a B/IP port: the port's own groups take an
/// Unconfirmed-Request as a broadcast; everything else is dropped.
#[test]
fn group_destinations_agree() {
    let mut scenario = two_ports();
    scenario.ports.push(ip6(3000));
    scenario.capacities.push(16);
    let own = &BIP_GROUPS[0];
    let other = &BIP_GROUPS[2];
    let mut reject = message(0x03, &[1, 0x13, 0x88]);
    reject.destination = address(1000, own);
    let mut directed = message(0x01, &[0x13, 0x88]);
    directed.destination = address(1000, other);
    directed.hop_count = 255;
    let steps = vec![
        frame(1, R7, &apdu_to(1000, own, UNCONFIRMED)),
        frame(0, NODE, &apdu_to(1000, own, UNCONFIRMED)),
        frame(1, R7, &apdu_to(1000, own, CONFIRMED)),
        frame(1, R7, &apdu_to(1000, other, UNCONFIRMED)),
        frame(2, &IP6_NODES[0], &directed),
        frame(2, &IP6_NODES[0], &reject),
        frame(0, NODE, &apdu_to(2000, &[0xFF], UNCONFIRMED)),
    ];
    assert_eq!(run(&scenario, &steps), (7, 0));
}

/// At the route cap, learning, updates and Acks stop where they did, and
/// an I-Could-Be-Router for a new network is the one documented change.
#[test]
fn a_full_table_agrees_but_for_i_could_be_router() {
    let mut scenario = two_ports();
    scenario.hardened = true;
    scenario.seeds = (10..264)
        .map(|network| Seed {
            network,
            port: 0,
            next_hop: NODE.to_vec(),
            mark: None,
        })
        .collect();
    let steps = vec![
        frame(1, R7, &message(0x01, &[0x13, 0x88, 0, 10])),
        frame(1, R7, &message(0x01, &[0, 10])),
        frame(1, R7, &message(0x01, &[0, 10])),
        frame(1, R7, &message(0x07, &[1, 0, 11, 1, 0])),
        frame(1, R7, &message(0x06, &[2, 0x13, 0x88, 1, 0, 0, 12, 2, 0])),
        frame(1, R7, &message(0x02, &[0, 12, 9])),
        frame(1, R7, &message(0x02, &[0x13, 0x88, 9])),
    ];
    assert_eq!(run(&scenario, &steps), (6, 1));
}

/// Hardened convergence: a second claim exactly 60 s after the first
/// still corroborates; one later starts over. Standard convergence warns
/// of a route that moves three times in a minute.
#[test]
fn corroboration_and_flapping_agree() {
    let mut scenario = two_ports();
    scenario.ports.push(ip6(3000));
    scenario.capacities.push(16);
    scenario.seeds = vec![Seed {
        network: 9,
        port: 0,
        next_hop: NODE.to_vec(),
        mark: None,
    }];
    let i_am = message(0x01, &[0, 9]);
    let claims = |hardened| {
        let mut scenario = scenario.clone();
        scenario.hardened = hardened;
        scenario
    };
    let steps = vec![
        frame(1, R7, &i_am),
        Step::Advance(60_000),
        frame(1, R7, &i_am),
        frame(2, &IP6_NODES[1], &i_am),
        Step::Advance(60_001),
        frame(2, &IP6_NODES[1], &i_am),
        Step::Advance(10_000),
        frame(1, R7, &i_am),
        frame(0, NODE, &i_am),
        frame(2, &IP6_NODES[1], &i_am),
    ];
    assert_eq!(run(&claims(true), &steps), (7, 0));
    assert_eq!(run(&claims(false), &steps), (7, 0));
}

/// The legacy copy's own startup on one port, with a priority NPDU.
#[test]
fn priority_and_one_port_agree() {
    let mut scenario = two_ports();
    scenario.ports.truncate(1);
    scenario.capacities.truncate(1);
    let mut urgent = apdu_to(1000, &[0x77], UNCONFIRMED);
    urgent.priority = NetworkPriority::LIFE_SAFETY;
    let steps = vec![frame(0, NODE, &urgent), frame(0, NODE, &message(0x00, &[]))];
    assert_eq!(run(&scenario, &steps), (2, 0));
}
