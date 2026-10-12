//! The B/IP transport on the node engine against the transport it replaced
//! (#1622).
//!
//! [`super::legacy`] keeps the old receive path, management exchanges and
//! registration worker. Each case runs one generated configuration and
//! script on both, under paused Tokio time on a socket that records what it
//! sends, and compares, after every step, the receive counters, the group
//! drops, the client snapshot (with the countdown to the next registration)
//! and the NPDUs handed up; and at the end every datagram sent, with its
//! time, and how each manual exchange ended, and when.
//!
//! A foreign device's BBMD is scripted: it answers each registration
//! attempt, or doesn't, as the case says. Timers keep a 10 ms grid: every
//! registration interval and timeout is a multiple of 10 ms, script events
//! land 5 ms off it, the BBMD's answers 3 ms off it, and manual exchanges
//! time out 2 ms off it, so nothing the script does falls at the instant a
//! timer fires, where the two sides' tasks may take turns in either order.
//!
//! Left out, each with a test of its own:
//! - a request the node refuses with a NAK, from a source no reply can
//!   reach (the unspecified address, port 0 or the node itself): the old
//!   transport answered it, and the node drops it (`unanswerable_tests.rs`);
//! - a wildcard bind whose broadcast address is its own address, the one
//!   case where the transport's group test (on the bound 0.0.0.0) and the
//!   node's (on its own address) differ;
//! - a manual Register, Delete-FDT-Entry or Write-BDT from a foreign device
//!   to its own BBMD, which the node refuses (`ManagedRegistration`);
//! - BBMD plus foreign device on one transport, which `start()` refuses;
//! - a foreign device's BBMD that no node can register with: unspecified,
//!   a group address, port 0 or the node itself.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4};
use std::time::Duration;

use proptest::prelude::*;
use tokio::time::Instant;

use super::differential_support::{
    Config, Ended, Engine, Incoming, Legacy, ModeConfig, Observation, Request, Side, OTHER_LOCAL,
    OWN_IP, SUBNET_BROADCAST,
};
use super::send_sink::Sent;
use super::ForeignDeviceConfig;
use crate::bvll::raw_frame;
use bacnet_bip_codec::{BvlcFunction, Bvll, BvllFrame};

const BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 1), 0xBAC0);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 30), 0xBAC0);

/// What the scripted BBMD does with one registration attempt.
#[derive(Clone, Copy, Debug)]
enum Reply {
    Silent,
    /// A BVLC-Result with `code`, `delay` ms after the attempt.
    Answer {
        code: u16,
        delay: u64,
    },
}

/// Where a datagram comes from.
#[derive(Clone, Copy, Debug)]
enum From {
    Address(SocketAddrV4),
    /// The node's own address.
    Own,
    /// The peer of the newest manual exchange, or [`PEER`].
    LastPeer,
}

/// One step of a script.
#[derive(Clone, Debug)]
enum Event {
    Datagram {
        payload: Vec<u8>,
        from: From,
        destination: IpAddr,
        os_group: Option<bool>,
        listener: Option<Option<u32>>,
        /// Sent this many times in a row, as a flood over the rate limit.
        repeat: u8,
    },
    Wait(u64),
    Begin(Request, SocketAddrV4),
    Sent,
    Fail,
    Cancel,
    Broadcast(Vec<u8>),
    Unicast(Vec<u8>, SocketAddrV4),
}

#[derive(Clone, Debug)]
struct Case {
    config: Config,
    replies: Vec<Reply>,
    events: Vec<Event>,
}

/// What a side did over a whole case.
#[derive(Debug, PartialEq, Eq)]
struct Trace {
    /// After each step: the step's own result, and the observation.
    steps: Vec<(String, Observation)>,
    /// Every datagram sent: microseconds from the start, destination,
    /// octets; in order of time, and by content within one instant.
    sent: Vec<(u128, SocketAddrV4, Vec<u8>)>,
    ended: Vec<Ended>,
}

/// Run `case` on a fresh side made by `make`, in a runtime of its own.
fn run<S: Side, F>(case: &Case, make: impl FnOnce(Config) -> F) -> Trace
where
    F: std::future::Future<Output = S>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(async {
        let side = make(case.config.clone()).await;
        drive(side, case).await
    })
}

/// The script's driver, the same for both sides.
struct Driver<'c, S> {
    side: S,
    case: &'c Case,
    start: Instant,
    watch: tokio::sync::mpsc::UnboundedReceiver<Sent>,
    /// The BBMD's answers due, in order of time.
    replies: Vec<(Instant, Incoming)>,
    attempts: usize,
    last_peer: SocketAddrV4,
}

impl<S: Side> Driver<'_, S> {
    /// Let time run to `target`, answering registration attempts as the
    /// script says.
    async fn advance(&mut self, target: Instant) {
        loop {
            let wake = self
                .replies
                .first()
                .map_or(target, |(at, _)| (*at).min(target));
            tokio::select! {
                biased;
                Some(sent) = self.watch.recv() => self.on_sent(&sent),
                () = tokio::time::sleep_until(wake) => {
                    while self.replies.first().is_some_and(|(at, _)| *at <= Instant::now()) {
                        let (_, incoming) = self.replies.remove(0);
                        self.side.receive(&incoming).await;
                    }
                    if Instant::now() >= target {
                        return;
                    }
                }
            }
        }
    }

    /// Whether the old transport answered `incoming` with a NAK the node
    /// doesn't send: a request it refuses, from a source no reply can reach.
    fn unanswerable(&self, incoming: &Incoming) -> bool {
        let (source, own) = (incoming.source, self.case.config.own());
        let unreachable = source.ip().is_unspecified() || source.port() == 0 || source == own;
        let Ok(frame) = BvllFrame::decode(&incoming.payload) else {
            return false;
        };
        if !unreachable || frame.function.nak().is_none() {
            return false;
        }
        match frame.message() {
            Err(_) | Ok(Bvll::WriteBroadcastDistributionTable(_)) => true,
            // Its own echo, either way.
            Ok(Bvll::DistributeBroadcastToNetwork(_)) if source == own => false,
            // A BBMD serves the other five, as before.
            Ok(_) => !matches!(self.case.config.mode, ModeConfig::Bbmd),
        }
    }

    fn on_sent(&mut self, sent: &Sent) {
        let Some(foreign) = self.case.config.foreign() else {
            return;
        };
        let bbmd = SocketAddrV4::new(foreign.bbmd_ip, foreign.bbmd_port);
        if sent.to != bbmd || sent.bytes.get(1) != Some(&0x05) {
            return;
        }
        let reply = self.case.replies.get(self.attempts).copied();
        self.attempts += 1;
        if let Some(Reply::Answer { code, delay }) = reply {
            let incoming = Incoming {
                payload: raw_frame(BvlcFunction::BVLC_RESULT, &code.to_be_bytes()).to_vec(),
                source: bbmd,
                destination: IpAddr::V4(OWN_IP),
                os_group: None,
                listener: None,
            };
            let at = sent.at + Duration::from_millis(delay);
            let position = self.replies.partition_point(|(due, _)| *due <= at);
            self.replies.insert(position, (at, incoming));
        }
    }

    async fn step(&mut self, event: &Event) -> String {
        match event {
            Event::Datagram {
                payload,
                from,
                destination,
                os_group,
                listener,
                repeat,
            } => {
                let source = match from {
                    From::Address(address) => *address,
                    From::Own => self.case.config.own(),
                    From::LastPeer => self.last_peer,
                };
                let incoming = Incoming {
                    payload: payload.clone(),
                    source,
                    destination: *destination,
                    os_group: *os_group,
                    listener: listener.filter(|_| self.case.config.listener),
                };
                if self.unanswerable(&incoming) {
                    return "left out".into();
                }
                for _ in 0..*repeat {
                    self.side.receive(&incoming).await;
                }
                String::new()
            }
            Event::Wait(_) => String::new(),
            Event::Begin(request, peer) => {
                self.last_peer = *peer;
                format!("{:?}", self.side.begin(*request, *peer))
            }
            Event::Sent => {
                self.side.sent();
                String::new()
            }
            Event::Fail => {
                self.side.fail();
                String::new()
            }
            Event::Cancel => {
                self.side.cancel().await;
                String::new()
            }
            Event::Broadcast(npdu) => format!("{:?}", self.side.broadcast(npdu).await),
            Event::Unicast(npdu, to) => format!("{:?}", self.side.unicast(npdu, *to).await),
        }
    }
}

async fn drive<S: Side>(side: S, case: &Case) -> Trace {
    let start = Instant::now();
    let watch = side.sink().watch();
    let mut driver = Driver {
        side,
        case,
        start,
        watch,
        replies: Vec::new(),
        attempts: 0,
        last_peer: PEER,
    };
    let mut at = start + Duration::from_millis(5);
    let mut steps = Vec::new();
    for event in &case.events {
        if let Event::Wait(ms) = event {
            at += Duration::from_millis(*ms);
        }
        driver.advance(at).await;
        let result = driver.step(event).await;
        // Let the tasks the step woke run, at the same instant.
        tokio::task::yield_now().await;
        steps.push((result, driver.side.observe()));
    }
    // Long enough for a registration timeout, the next attempt and a
    // manual exchange's timeout to show.
    driver.advance(at + Duration::from_millis(7000)).await;
    steps.push((String::new(), driver.side.observe()));
    let mut sent: Vec<_> = driver
        .side
        .sink()
        .take()
        .into_iter()
        .map(|sent| ((sent.at - driver.start).as_micros(), sent.to, sent.bytes))
        .collect();
    sent.sort();
    Trace {
        steps,
        sent,
        ended: driver.side.ended(),
    }
}

/// Run `case` on both sides and compare, naming the first step that differs.
fn compare(case: &Case) -> Result<(), TestCaseError> {
    let legacy = run(case, |config| async move { Legacy::new(&config).await });
    let engine = run(case, |config| async move { Engine::new(&config).await });
    for (index, (old, new)) in legacy.steps.iter().zip(&engine.steps).enumerate() {
        let event = case.events.get(index);
        prop_assert_eq!(old, new, "step {} ({:?})", index, event);
    }
    prop_assert_eq!(&legacy.sent, &engine.sent, "sends");
    prop_assert_eq!(&legacy.ended, &engine.ended, "exchange ends");
    prop_assert_eq!(legacy, engine);
    Ok(())
}

fn address(ip: [u8; 4], port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(ip), port)
}

fn foreign_config() -> impl Strategy<Value = ModeConfig> {
    let bbmd = prop_oneof![Just(BBMD), Just(address([192, 0, 2, 40], 0xBAC1))];
    let ttl_interval = prop_oneof![
        Just((2u16, None)),
        Just((2, Some(300))),
        Just((5, Some(1000))),
        Just((5, Some(50))),
        Just((60, None)),
        Just((2, Some(0))),
    ];
    (bbmd, ttl_interval).prop_map(|(bbmd, (ttl, interval))| {
        ModeConfig::Foreign(ForeignDeviceConfig {
            bbmd_ip: *bbmd.ip(),
            bbmd_port: bbmd.port(),
            ttl,
            renewal_interval: interval.map(|ms| match ms {
                // A nanosecond, which the 100 ms floor raises.
                0 => Duration::from_nanos(1),
                ms => Duration::from_millis(ms),
            }),
        })
    })
}

fn config() -> impl Strategy<Value = Config> {
    let mode = prop_oneof![
        3 => Just(ModeConfig::Normal),
        3 => foreign_config(),
        2 => Just(ModeConfig::Bbmd),
    ];
    (
        prop_oneof![Just(0xBAC0u16), Just(0xBAC1)],
        0u8..3,
        any::<bool>(),
        any::<bool>(),
        mode,
    )
        .prop_map(|(own_port, broadcast, wildcard, listener, mode)| {
            let broadcast = match broadcast {
                0 => SUBNET_BROADCAST,
                1 => Ipv4Addr::BROADCAST,
                // The one case where the groups differ is left out.
                _ if wildcard => SUBNET_BROADCAST,
                _ => OWN_IP,
            };
            Config {
                own_port,
                broadcast,
                wildcard,
                listener,
                mode,
            }
        })
}

fn reply() -> impl Strategy<Value = Reply> {
    let code = prop_oneof![Just(0u16), Just(0x0030), Just(0x0060), Just(0x1234)];
    let delay = prop_oneof![Just(3u64), Just(13), Just(253), Just(3013)];
    prop_oneof![
        1 => Just(Reply::Silent),
        3 => (code, delay).prop_map(|(code, delay)| Reply::Answer { code, delay }),
    ]
}

/// Addresses a datagram, an origin or a request may name.
fn endpoint(own_port: u16) -> impl Strategy<Value = From> {
    let at = |ip: [u8; 4], port: u16| Just(From::Address(address(ip, port)));
    prop_oneof![
        4 => at([192, 0, 2, 30], 0xBAC0),
        1 => at([192, 0, 2, 30], 0xBAC1),
        2 => at([198, 51, 100, 7], 0xBAC0),
        2 => Just(From::Address(BBMD)),
        1 => at([192, 0, 2, 40], 0xBAC1),
        1 => at([203, 0, 113, 9], 0xBAC0),
        2 => Just(From::Own),
        1 => at([192, 0, 2, 10], own_port ^ 1),
        1 => at([255, 255, 255, 255], 0xBAC0),
        1 => at([224, 0, 0, 1], 0xBAC0),
        1 => at([192, 0, 2, 255], own_port),
        1 => at([0, 0, 0, 0], 0xBAC0),
        1 => at([192, 0, 2, 30], 0),
        3 => Just(From::LastPeer),
    ]
}

fn octets(from: From, own_port: u16) -> Vec<u8> {
    let address = match from {
        From::Address(address) => address,
        From::Own => address(OWN_IP.octets(), own_port),
        From::LastPeer => PEER,
    };
    let mut octets = address.ip().octets().to_vec();
    octets.extend_from_slice(&address.port().to_be_bytes());
    octets
}

/// A frame of every kind: well formed, with a payload that doesn't fit its
/// function, or with a damaged header.
fn payload(own_port: u16) -> impl Strategy<Value = Vec<u8>> {
    let npdu = prop::collection::vec(any::<u8>(), 0..4);
    let code = prop_oneof![
        Just(0u16),
        Just(0x10),
        Just(0x20),
        Just(0x30),
        Just(0x40),
        Just(0x50),
        Just(0x60),
        Just(0x1234)
    ];
    let row = [192u8, 0, 2, 40, 0xBA, 0xC0, 255, 255, 255, 255];
    let fdt_row = [192u8, 0, 2, 40, 0xBA, 0xC0, 0, 60, 0, 90];
    let frame = |function: u8, payload: Vec<u8>| {
        raw_frame(BvlcFunction::from_raw(function), &payload).to_vec()
    };
    prop_oneof![
        3 => code.prop_map(move |code| frame(0x00, code.to_be_bytes().to_vec())),
        1 => Just(frame(0x01, row.to_vec())),
        1 => Just(frame(0x02, vec![])),
        2 => prop_oneof![Just(0usize), Just(1), Just(129)]
            .prop_map(move |rows| frame(0x03, row.repeat(rows))),
        2 => (endpoint(own_port), npdu.clone())
            .prop_map(move |(origin, npdu)| frame(0x04, [octets(origin, own_port), npdu].concat())),
        1 => prop_oneof![Just(0u16), Just(60)].prop_map(move |ttl| frame(0x05, ttl.to_be_bytes().to_vec())),
        1 => Just(frame(0x06, vec![])),
        2 => prop_oneof![Just(0usize), Just(1), Just(129)]
            .prop_map(move |rows| frame(0x07, fdt_row.repeat(rows))),
        1 => endpoint(own_port).prop_map(move |entry| frame(0x08, octets(entry, own_port))),
        2 => npdu.clone().prop_map(move |npdu| frame(0x09, npdu)),
        3 => npdu.clone().prop_map(move |npdu| frame(0x0A, npdu)),
        3 => npdu.prop_map(move |npdu| frame(0x0B, npdu)),
        3 => (0u8..16, prop::collection::vec(any::<u8>(), 0..7))
            .prop_map(move |(function, payload)| frame(function, payload)),
        1 => prop_oneof![
            Just(vec![0x81, 0x0A]),
            Just(vec![0x82, 0x0A, 0x00, 0x04]),
            Just(vec![0x81, 0x0A, 0x00, 0x09, 0x01]),
        ],
    ]
}

/// A response to one of the node's own requests: a BVLC-Result, or a
/// table ACK, well formed, malformed or over the table limit.
fn response() -> impl Strategy<Value = Vec<u8>> {
    let code = prop_oneof![
        Just(0u16),
        Just(0x10),
        Just(0x20),
        Just(0x30),
        Just(0x40),
        Just(0x50),
        Just(0x60),
        Just(0x1234)
    ];
    let bdt_row = [192u8, 0, 2, 40, 0xBA, 0xC0, 255, 255, 255, 255];
    let fdt_row = [192u8, 0, 2, 40, 0xBA, 0xC0, 0, 60, 0, 90];
    let rows = prop_oneof![Just(0usize), Just(2), Just(129)];
    let frame = |function: u8, payload: &[u8]| {
        raw_frame(BvlcFunction::from_raw(function), payload).to_vec()
    };
    prop_oneof![
        4 => code.prop_map(move |code| frame(0x00, &code.to_be_bytes())),
        1 => Just(frame(0x00, &[0])),
        2 => rows.clone().prop_map(move |rows| frame(0x03, &bdt_row.repeat(rows))),
        2 => rows.prop_map(move |rows| frame(0x07, &fdt_row.repeat(rows))),
        1 => Just(frame(0x03, &[0; 3])),
        1 => Just(frame(0x07, &[0; 4])),
    ]
}

fn destination() -> impl Strategy<Value = IpAddr> {
    prop_oneof![
        6 => Just(IpAddr::V4(OWN_IP)),
        3 => Just(IpAddr::V4(SUBNET_BROADCAST)),
        2 => Just(IpAddr::V4(Ipv4Addr::BROADCAST)),
        1 => Just(IpAddr::V4(OTHER_LOCAL)),
        1 => Just(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 12))),
        1 => Just(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))),
        1 => Just(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ]
}

fn request(foreign: Option<SocketAddrV4>) -> impl Strategy<Value = Event> {
    let request = prop_oneof![
        Just(Request::ReadBdt),
        Just(Request::WriteBdt),
        Just(Request::ReadFdt),
        Just(Request::DeleteFdtEntry),
        Just(Request::Register(0)),
        Just(Request::Register(60)),
    ];
    let peer = prop_oneof![
        Just(PEER),
        Just(BBMD),
        Just(address([198, 51, 100, 7], 0xBAC0)),
        Just(address([192, 0, 2, 40], 0xBAC1)),
    ];
    (request, peer).prop_map(move |(request, peer)| {
        // A foreign device refuses these to its own BBMD (left out).
        let managed = !matches!(request, Request::ReadBdt | Request::ReadFdt);
        let request = if managed && foreign == Some(peer) {
            Request::ReadBdt
        } else {
            request
        };
        Event::Begin(request, peer)
    })
}

fn event(config: &Config) -> impl Strategy<Value = Event> {
    let own_port = config.own_port;
    let foreign = config
        .foreign()
        .map(|foreign| SocketAddrV4::new(foreign.bbmd_ip, foreign.bbmd_port));
    let os_group = prop_oneof![6 => Just(None), 1 => Just(Some(true)), 1 => Just(Some(false))];
    let listener = prop_oneof![
        4 => Just(None),
        1 => Just(Some(Some(2u32))),
        1 => Just(Some(Some(3))),
        1 => Just(Some(None)),
    ];
    let repeat = prop_oneof![12 => Just(1u8), 1 => Just(17)];
    let datagram = (
        payload(own_port),
        endpoint(own_port),
        destination(),
        os_group,
        listener,
        repeat,
    )
        .prop_map(
            |(payload, from, destination, os_group, listener, repeat)| Event::Datagram {
                payload,
                from,
                destination,
                os_group,
                listener,
                repeat,
            },
        );
    // An answer from the newest manual exchange's peer, to this node.
    let respond = response().prop_map(|payload| Event::Datagram {
        payload,
        from: From::LastPeer,
        destination: IpAddr::V4(OWN_IP),
        os_group: None,
        listener: None,
        repeat: 1,
    });
    let wait = prop_oneof![
        Just(10u64),
        Just(20),
        Just(100),
        Just(250),
        Just(1000),
        Just(3000),
        Just(10000),
    ];
    let npdu = prop::collection::vec(any::<u8>(), 0..4);
    prop_oneof![
        12 => datagram,
        8 => respond,
        5 => wait.prop_map(Event::Wait),
        3 => request(foreign),
        4 => Just(Event::Sent),
        1 => Just(Event::Fail),
        1 => Just(Event::Cancel),
        1 => npdu.clone().prop_map(Event::Broadcast),
        1 => npdu.prop_map(|npdu| Event::Unicast(npdu, PEER)),
    ]
}

fn case() -> impl Strategy<Value = Case> {
    config().prop_flat_map(|config| {
        let events = prop::collection::vec(event(&config), 1..48);
        let replies = prop::collection::vec(reply(), 0..12);
        (Just(config), replies, events).prop_map(|(config, replies, events)| Case {
            config,
            replies,
            events,
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn the_engine_transport_does_what_the_old_one_did(case in case()) {
        compare(&case)?;
    }
}

/// A fixed configuration: the node at 192.0.2.10:47808 on 192.0.2.0/24.
fn fixed(mode: ModeConfig) -> Config {
    Config {
        own_port: 0xBAC0,
        broadcast: SUBNET_BROADCAST,
        wildcard: false,
        listener: false,
        mode,
    }
}

fn from_peer(payload: Vec<u8>) -> Event {
    Event::Datagram {
        payload,
        from: From::LastPeer,
        destination: IpAddr::V4(OWN_IP),
        os_group: None,
        listener: None,
        repeat: 1,
    }
}

fn result(code: u16) -> Vec<u8> {
    raw_frame(BvlcFunction::BVLC_RESULT, &code.to_be_bytes()).to_vec()
}

fn ack(function: BvlcFunction, row: [u8; 10], rows: usize) -> Vec<u8> {
    raw_frame(function, &row.repeat(rows)).to_vec()
}

/// Each request answered every way: before and after its send, by its own
/// NAK, another's, success and its ACK, malformed and oversized ACKs, late,
/// and given up or failed.
#[test]
fn every_exchange_ends_as_it_did() {
    use Event::{Begin, Cancel, Fail, Sent, Wait};
    let bdt_row = [192, 0, 2, 40, 0xBA, 0xC0, 255, 255, 255, 255];
    let fdt_row = [192, 0, 2, 40, 0xBA, 0xC0, 0, 60, 0, 90];
    let bdt_ack = BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK;
    let fdt_ack = BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK;
    let mut events = Vec::new();
    for (request, answers) in [
        (Request::ReadBdt, vec![result(0x40), result(0x20)]),
        (Request::ReadBdt, vec![ack(bdt_ack, bdt_row, 2)]),
        (Request::ReadBdt, vec![ack(bdt_ack, bdt_row, 129)]),
        (Request::ReadBdt, vec![raw_frame(bdt_ack, &[0; 3]).to_vec()]),
        (Request::ReadFdt, vec![result(0), ack(fdt_ack, fdt_row, 1)]),
        (Request::ReadFdt, vec![ack(fdt_ack, fdt_row, 129)]),
        (Request::WriteBdt, vec![result(0x20), result(0x10)]),
        (Request::DeleteFdtEntry, vec![result(0x1234), result(0)]),
        (Request::Register(60), vec![result(0x60), result(0x30)]),
        (Request::Register(0), vec![result(0)]),
    ] {
        // Answered after its send, then before it.
        for early in [false, true] {
            events.push(Begin(request, PEER));
            if !early {
                events.push(Sent);
            }
            events.extend(answers.iter().cloned().map(from_peer));
            if early {
                events.push(Sent);
            }
            events.push(Wait(10));
        }
    }
    // A timeout, then its late answer; a failed send; and given up before
    // and after the send.
    events.extend([
        Begin(Request::Register(60), PEER),
        Sent,
        Wait(3010),
        from_peer(result(0)),
        Begin(Request::ReadBdt, PEER),
        Fail,
        Begin(Request::ReadFdt, PEER),
        Cancel,
        Begin(Request::ReadFdt, PEER),
        Sent,
        Cancel,
        from_peer(ack(fdt_ack, fdt_row, 0)),
    ]);
    let case = Case {
        config: fixed(ModeConfig::Normal),
        replies: Vec::new(),
        events,
    };
    compare(&case).unwrap();
}

/// A foreign device's registrations, answered, refused, late and lost, with
/// broadcasts and a manual request between them.
#[test]
fn registration_runs_on_the_same_schedule() {
    use Event::{Begin, Broadcast, Sent, Wait};
    let foreign = ModeConfig::Foreign(ForeignDeviceConfig {
        bbmd_ip: *BBMD.ip(),
        bbmd_port: BBMD.port(),
        ttl: 5,
        renewal_interval: Some(Duration::from_millis(1000)),
    });
    let replies = vec![
        Reply::Answer { code: 0, delay: 13 },
        Reply::Silent,
        Reply::Answer {
            code: 0x30,
            delay: 253,
        },
        Reply::Answer {
            code: 0,
            delay: 1003,
        },
        Reply::Answer {
            code: 0x60,
            delay: 3,
        },
        Reply::Silent,
        Reply::Answer { code: 0, delay: 3 },
    ];
    let events = vec![
        Broadcast(vec![1, 0]),
        Wait(20),
        // Holds the one slot over the next attempt, which finds it busy.
        Begin(Request::ReadBdt, BBMD),
        Sent,
        Wait(3000),
        Broadcast(vec![1, 0]),
        Wait(10000),
    ];
    let case = Case {
        config: fixed(foreign),
        replies,
        events,
    };
    compare(&case).unwrap();
}
