//! The two routers the cutover's differential test runs side by side
//! (#1679): the legacy copy of the hosted router ([`super::legacy`]) and the
//! hosted router on the `bacnet-router-core` engine, each with its own port
//! queues, local receivers and gate, fed the same frames on one paused
//! Tokio clock. [`Pair::check`] compares everything either can show.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_router_core::{Mac, PortId};
use bacnet_transport::port::GroupDestinations;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;

use super::control_policy::{ControlAuthContext, ControlAuthorizer, ControlGate, ControlPolicy};
use super::dispatch::{Outputs, PortDispatch, Shared};
use super::engine::{Engine, RouterClock};
use super::hosted_host::PortGroups;
use super::legacy;
use super::local_control::LocalControl;
use super::{actions, SendRequest};
use crate::layer::{AdmissionReceiver, ReceivedApdu, ReceivedNetworkControl};

/// The router's B/IP port number, the UDP port of its own groups.
pub(super) const BIP_PORT: [u8; 2] = [0xBA, 0xC0];

/// What groups a port's transport knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Groups {
    /// None, as a custom transport that keeps the default.
    None,
    /// B/IP: the all-ones and multicast addresses at any UDP port, of which
    /// those at [`BIP_PORT`] are the port's own.
    Bip,
    /// MS/TP: 0xFF.
    Mstp,
}

impl Groups {
    fn rules(self) -> (GroupDestinations, GroupDestinations) {
        match self {
            Self::None => (GroupDestinations::unknown(), GroupDestinations::unknown()),
            Self::Bip => {
                let group = |mac: &[u8]| {
                    mac.len() == 6 && (mac[..4] == [255; 4] || (224..=239).contains(&mac[0]))
                };
                (
                    GroupDestinations::new(group),
                    GroupDestinations::new(move |mac| group(mac) && mac[4..] == BIP_PORT),
                )
            }
            Self::Mstp => {
                let group = |mac: &[u8]| mac == [0xFF];
                (GroupDestinations::new(group), GroupDestinations::new(group))
            }
        }
    }
}

/// One router port.
#[derive(Debug, Clone)]
pub(super) struct PortSpec {
    pub(super) network: u16,
    pub(super) mac: Vec<u8>,
    pub(super) groups: Groups,
}

/// A learned route both tables start with.
#[derive(Debug, Clone)]
pub(super) struct Seed {
    pub(super) network: u16,
    pub(super) port: usize,
    pub(super) next_hop: Vec<u8>,
    /// `Some(Some(ms))`: busy for that long; `Some(None)`: unreachable.
    pub(super) mark: Option<Option<u64>>,
}

/// How the wire-control gate decides.
#[derive(Debug, Clone, Copy)]
pub(super) enum Gate {
    Permissive,
    Hardened,
    /// An authorizer that refuses a message whose type's bit is set in the
    /// mask, or that names an odd network when the low bit is set, under
    /// the given policy.
    Authorizer {
        mask: u32,
        hardened: bool,
    },
}

impl Gate {
    /// A gate as this says, recording each context its authorizer sees.
    fn build(self, asked: &Arc<StdMutex<Vec<ControlAuthContext>>>) -> Arc<ControlGate> {
        Arc::new(match self {
            Self::Permissive => ControlGate::permissive(),
            Self::Hardened => ControlGate::hardened(),
            Self::Authorizer { mask, hardened } => {
                let asked = Arc::clone(asked);
                let authorizer: ControlAuthorizer = Arc::new(move |ctx: &ControlAuthContext| {
                    asked.lock().unwrap().push(ctx.clone());
                    let refused_type = (mask >> (1 + (ctx.message_type & 15))) & 1 != 0;
                    let odd = mask & 1 != 0 && ctx.target_nets.iter().any(|net| net % 2 == 1);
                    !refused_type && !odd
                });
                let policy = if hardened {
                    ControlPolicy::Hardened
                } else {
                    ControlPolicy::Permissive
                };
                ControlGate::new(policy, Some(authorizer))
            }
        })
    }
}

/// Everything a run starts from.
#[derive(Debug, Clone)]
pub(super) struct Scenario {
    pub(super) ports: Vec<PortSpec>,
    pub(super) seeds: Vec<Seed>,
    /// Rejects both tables take at the start: network, port, reason.
    pub(super) holds: Vec<(u16, usize, u8)>,
    pub(super) hardened: bool,
    pub(super) gate: Gate,
    /// Each port's send queue capacity.
    pub(super) capacities: Vec<usize>,
}

/// A frame a port receives.
#[derive(Debug, Clone)]
pub(super) struct Arrival {
    pub(super) port: usize,
    pub(super) source: Vec<u8>,
    pub(super) link_group: bool,
    pub(super) npdu: Vec<u8>,
    pub(super) attributes: bool,
    pub(super) reply: bool,
}

/// One step of a run.
#[derive(Debug, Clone)]
pub(super) enum Step {
    Frame(Arrival),
    /// Let this many milliseconds pass, sweeping at each sweep deadline.
    Advance(u64),
    /// Stop reading a port's queue, so it can fill.
    Clog(usize),
    /// Read it again.
    Unclog(usize),
}

#[path = "differential_views.rs"]
mod views;
use views::*;

/// The legacy copy, as `start_dispatch` set it up.
struct Legacy {
    table: Arc<tokio::sync::Mutex<legacy::router_table::RouterTable>>,
    discovery: Arc<tokio::sync::Mutex<legacy::DiscoveryTracker>>,
    ports: Vec<legacy::dispatch::PortDispatch>,
    rxs: Vec<mpsc::Receiver<SendRequest>>,
    apdus: AdmissionReceiver<ReceivedApdu>,
    controls: AdmissionReceiver<ReceivedNetworkControl>,
    gate: Arc<ControlGate>,
    drops: [Arc<AtomicU64>; 4],
    sequence: Arc<AtomicU64>,
    asked: Arc<StdMutex<Vec<ControlAuthContext>>>,
}

/// The hosted router on the engine.
struct Hosted {
    shared: Arc<Shared>,
    ports: Vec<PortDispatch>,
    rxs: Vec<mpsc::Receiver<SendRequest>>,
    apdus: AdmissionReceiver<ReceivedApdu>,
    controls: AdmissionReceiver<ReceivedNetworkControl>,
    gate: Arc<ControlGate>,
    sequence: Arc<AtomicU64>,
    asked: Arc<StdMutex<Vec<ControlAuthContext>>>,
}

/// The two routers, run side by side.
pub(super) struct Pair {
    origin: tokio::time::Instant,
    legacy: Legacy,
    hosted: Hosted,
    networks: Vec<u16>,
    clogged: Vec<bool>,
    /// Steps skipped as a documented change (an I-Could-Be-Router at the
    /// route cap).
    pub(super) excluded: usize,
    /// Every frame step applied.
    pub(super) frames: usize,
    /// The sends the legacy copy's queues refused so far.
    refused: u64,
}

fn queues(
    capacities: &[usize],
) -> (
    Vec<mpsc::Sender<SendRequest>>,
    Vec<mpsc::Receiver<SendRequest>>,
) {
    capacities
        .iter()
        .map(|&capacity| mpsc::channel(capacity))
        .unzip()
}

impl Pair {
    /// Both routers on `scenario`, started, on the current (paused) Tokio
    /// time, with the startup announcements compared.
    pub(super) async fn start(scenario: &Scenario) -> Self {
        let _ = legacy::take_sent();
        let _ = legacy::take_refused();
        let _ = actions::take_sent();
        let origin = tokio::time::Instant::now();
        let networks: Vec<u16> = scenario.ports.iter().map(|port| port.network).collect();
        let rules: Vec<_> = scenario
            .ports
            .iter()
            .map(|port| port.groups.rules())
            .collect();

        // The legacy copy.
        let asked_l = Arc::default();
        let gate_l = scenario.gate.build(&asked_l);
        let mut table = if scenario.hardened {
            legacy::router_table::RouterTable::new_hardened()
        } else {
            legacy::router_table::RouterTable::new()
        };
        for (index, &network) in networks.iter().enumerate() {
            table.add_direct(network, index);
        }
        let (send_l, rxs_l) = queues(&scenario.capacities);
        let (local_l, apdus_l, counters) = AdmissionReceiver::channel(true);
        let apdus_l = AdmissionReceiver::from_apdu_parts(apdus_l, counters);
        let (control_l, controls_l, counters) = AdmissionReceiver::channel(true);
        let controls_l = AdmissionReceiver::from_parts(controls_l, counters);
        let drops: [Arc<AtomicU64>; 4] = Default::default();
        let sequence_l = Arc::new(AtomicU64::new(0));
        let own = legacy::local_control::OwnAddresses::new(
            scenario
                .ports
                .iter()
                .map(|port| NpduAddress {
                    network: port.network,
                    mac_address: MacAddr::from_slice(&port.mac),
                })
                .collect(),
        );
        let local_control = Arc::new(
            legacy::local_control::LocalControl::new(own, Some(control_l), Arc::clone(&sequence_l))
                .with_groups(legacy::group_dadr::GroupRules::new(
                    rules.iter().map(|rule| rule.0.clone()).collect(),
                    rules.iter().map(|rule| rule.1.clone()).collect(),
                    Arc::clone(&drops[3]),
                )),
        );
        let table = Arc::new(tokio::sync::Mutex::new(table));
        let discovery = Arc::<tokio::sync::Mutex<legacy::DiscoveryTracker>>::default();
        let send_l = Arc::new(send_l);
        legacy::announce(&send_l, &networks);
        let ports_l = (0..networks.len())
            .map(|index| legacy::dispatch::PortDispatch {
                table: Arc::clone(&table),
                discovery: Arc::clone(&discovery),
                control: Arc::clone(&gate_l),
                local_control: Arc::clone(&local_control),
                address_length_drops: Arc::clone(&drops[0]),
                global_broadcast_dadr_drops: Arc::clone(&drops[1]),
                broadcast_pdu_type_drops: Arc::clone(&drops[2]),
                local_tx: local_l.clone(),
                send_txs: Arc::clone(&send_l),
                port_idx: index,
                port_network: networks[index],
                local_mac: MacAddr::from_slice(&scenario.ports[index].mac),
            })
            .collect();

        // The hosted router on the engine.
        let asked_h = Arc::default();
        let gate_h = scenario.gate.build(&asked_h);
        let configs: Vec<_> = scenario
            .ports
            .iter()
            .map(|port| Engine::port(port.network, Mac::from_slice(&port.mac).unwrap()))
            .collect();
        let engine = Engine::new(&configs, RouterClock::start()).unwrap();
        if scenario.hardened {
            engine
                .lock()
                .table_mut()
                .set_convergence(bacnet_router_core::table::Convergence::Hardened);
        }
        let (send_h, rxs_h) = queues(&scenario.capacities);
        let (local_h, apdus_h, counters) = AdmissionReceiver::channel(true);
        let apdus_h = AdmissionReceiver::from_apdu_parts(apdus_h, counters);
        let (control_h, controls_h, counters) = AdmissionReceiver::channel(true);
        let controls_h = AdmissionReceiver::from_parts(controls_h, counters);
        let sequence_h = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            engine,
            gate: Arc::clone(&gate_h),
            groups: rules
                .into_iter()
                .map(|(groups, local_broadcasts)| PortGroups {
                    groups,
                    local_broadcasts,
                })
                .collect(),
            networks: networks.clone(),
        });
        shared.announce(&send_h);
        let outputs = Arc::new(Outputs {
            send_txs: send_h,
            local_tx: local_h,
            local_control: LocalControl::new(Some(control_h), Arc::clone(&sequence_h)),
        });
        let ports_h = (0..networks.len())
            .map(|index| PortDispatch {
                shared: Arc::clone(&shared),
                outputs: Arc::clone(&outputs),
                port: index as u8,
            })
            .collect();

        let mut pair = Self {
            origin,
            legacy: Legacy {
                table,
                discovery,
                ports: ports_l,
                rxs: rxs_l,
                apdus: apdus_l,
                controls: controls_l,
                gate: gate_l,
                drops,
                sequence: sequence_l,
                asked: asked_l,
            },
            hosted: Hosted {
                shared,
                ports: ports_h,
                rxs: rxs_h,
                apdus: apdus_h,
                controls: controls_h,
                gate: gate_h,
                sequence: sequence_h,
                asked: asked_h,
            },
            clogged: vec![false; networks.len()],
            networks,
            excluded: 0,
            frames: 0,
            refused: 0,
        };
        pair.seed(scenario).await;
        pair.check("start").await;
        pair
    }

    /// Give both tables the scenario's learned routes, marks and holds.
    async fn seed(&mut self, scenario: &Scenario) {
        let now_l = tokio::time::Instant::now();
        let now_h = self.hosted.shared.engine.now();
        let timing = bacnet_router_core::table::Timing::DEFAULT;
        let mut legacy = self.legacy.table.lock().await;
        let mut engine = self.hosted.shared.engine.lock();
        let hosted = engine.table_mut();
        for seed in &scenario.seeds {
            let mac = Mac::from_slice(&seed.next_hop).unwrap();
            legacy.add_learned_at(
                seed.network,
                seed.port,
                MacAddr::from_slice(&seed.next_hop),
                now_l,
            );
            hosted.add_learned(seed.network, PortId(seed.port as u8), mac, now_h);
            match seed.mark {
                Some(Some(ms)) => {
                    let busy = std::time::Duration::from_millis(ms);
                    legacy.mark_busy(seed.network, now_l + busy);
                    hosted.mark_busy(
                        seed.network,
                        now_h + bacnet_router_core::Duration::from_millis(ms),
                    );
                }
                Some(None) => {
                    legacy.mark_unreachable(seed.network);
                    hosted.mark_unreachable(seed.network);
                }
                None => {}
            }
        }
        for &(network, port, reason) in &scenario.holds {
            legacy.apply_reject(network, port, reason, now_l);
            hosted.apply_reject(network, PortId(port as u8), reason, now_h, &timing);
        }
    }

    /// Apply `step` to both routers, then compare them.
    pub(super) async fn step(&mut self, step: &Step) {
        match step {
            Step::Frame(arrival) => {
                if self.excluded_change(arrival).await {
                    self.excluded += 1;
                    return;
                }
                self.frames += 1;
                let port = arrival.port % self.networks.len();
                let (legacy, hosted) = (received(arrival), received(arrival));
                self.legacy.ports[port].dispatch(legacy).await;
                self.hosted.ports[port].dispatch(hosted);
            }
            Step::Advance(ms) => {
                let target = tokio::time::Instant::now() + std::time::Duration::from_millis(*ms);
                loop {
                    let clock = self.hosted.shared.engine.clock();
                    let next = self.hosted.shared.engine.lock().next_deadline();
                    let Some(due) = next
                        .map(|next| clock.instant(next))
                        .filter(|due| *due <= target)
                    else {
                        break;
                    };
                    tokio::time::advance(due - tokio::time::Instant::now()).await;
                    legacy::sweep(&self.legacy.table, &self.legacy.discovery).await;
                    self.hosted.shared.sweep();
                    self.check("sweep").await;
                }
                tokio::time::advance(target - tokio::time::Instant::now()).await;
            }
            Step::Clog(port) => self.clogged[port % self.networks.len()] = true,
            Step::Unclog(port) => self.clogged[port % self.networks.len()] = false,
        }
        self.check(&format!("{step:?}")).await;
    }

    /// Whether `arrival` is an I-Cou-Be-Router for a new network while the
    /// legacy table is at the route cap: the documented change, where the
    /// engine refuses what the legacy router stored.
    async fn excluded_change(&self, arrival: &Arrival) -> bool {
        let Ok(npdu) = decode_npdu(Bytes::copy_from_slice(&arrival.npdu)) else {
            return false;
        };
        if npdu.message_type != Some(0x02) || npdu.payload.len() != 3 {
            return false;
        }
        let network = u16::from_be_bytes([npdu.payload[0], npdu.payload[1]]);
        let table = self.legacy.table.lock().await;
        network != 0
            && network != 0xFFFF
            && table.lookup(network).is_none()
            && table.len() >= crate::router_table::ROUTE_CAP
    }

    /// Compare everything the two routers show, or panic saying where they
    /// part.
    pub(super) async fn check(&mut self, after: &str) {
        // Sends, in order and with their kinds, as each went into a queue.
        let sends_l: Vec<_> = legacy::take_sent()
            .into_iter()
            .map(|(npdu, kind)| sent(&npdu, legacy_kind(kind)))
            .collect();
        let sends_h: Vec<_> = actions::take_sent()
            .into_iter()
            .map(|(npdu, kind)| sent(&npdu, kind))
            .collect();
        assert_eq!(
            sends_l, sends_h,
            "sends (in order, with kinds) after {after}"
        );
        // What each open port queue holds.
        for port in 0..self.networks.len() {
            if self.clogged[port] {
                continue;
            }
            let wire_l = drain_wire(&mut self.legacy.rxs[port]);
            let wire_h = drain_wire(&mut self.hosted.rxs[port]);
            assert_eq!(wire_l, wire_h, "port {port} queue after {after}");
        }
        // Deliveries and controls.
        let apdus_l: Vec<Delivered> = drain_queue(&mut self.legacy.apdus);
        let apdus_h: Vec<Delivered> = drain_queue(&mut self.hosted.apdus);
        assert_eq!(apdus_l, apdus_h, "local deliveries after {after}");
        let controls_l: Vec<Control> = drain_queue(&mut self.legacy.controls);
        let controls_h: Vec<Control> = drain_queue(&mut self.hosted.controls);
        assert_eq!(controls_l, controls_h, "controls after {after}");
        // What the authorizers were asked, and the counts.
        let asked_l = std::mem::take(&mut *self.legacy.asked.lock().unwrap());
        let asked_h = std::mem::take(&mut *self.hosted.asked.lock().unwrap());
        assert_eq!(asked_l, asked_h, "authorizer contexts after {after}");
        assert_eq!(
            self.legacy_counts().await,
            self.hosted_counts(),
            "counts after {after}"
        );
        // The tables and the searches.
        assert_eq!(
            self.legacy_routes().await,
            self.hosted_routes(),
            "tables after {after}"
        );
        let searches_l = self.legacy_searches().await;
        let engine = self.hosted.shared.engine.lock();
        let discovery = engine.discovery();
        assert_eq!(
            (
                searches_l.len(),
                searches_l.is_empty() && discovery.is_cancelled()
            ),
            (discovery.len(), discovery.is_cancelled()),
            "searches after {after}"
        );
        for (network, at) in searches_l {
            assert_eq!(
                discovery.last_solicited(network).map(|at| at.as_nanos()),
                Some(at),
                "search for {network} after {after}"
            );
        }
    }

    fn nanos(&self, at: tokio::time::Instant) -> u64 {
        u64::try_from(at.duration_since(self.origin).as_nanos()).unwrap()
    }

    async fn legacy_counts(&mut self) -> Counts {
        self.refused += legacy::take_refused();
        let claims = self.legacy.table.lock().await.claim_snapshot();
        let mut counts: Counts = self
            .legacy
            .drops
            .iter()
            .map(|drop| drop.load(Ordering::Relaxed))
            .collect();
        counts.extend([
            claims.reject_applied,
            claims.reject_dampened,
            claims.reject_dampened_same_state,
            claims.reject_dampened_hold_down,
            claims.learned_ok,
            claims.learned_cap_ignored,
            claims.flap_warned,
            claims.pending_started,
            claims.corroborated_applied,
            claims.pending_expired,
            claims.disconnect_removal_ignored,
            0,
            self.legacy.sequence.load(Ordering::SeqCst),
            self.refused,
        ]);
        counts.extend(admission(&self.legacy.apdus, &self.legacy.controls));
        counts.extend(decisions(&self.legacy.gate));
        counts
    }

    fn hosted_counts(&self) -> Counts {
        let engine = self.hosted.shared.engine.lock();
        let drops = engine.counters();
        let claims = engine.table().claim_counters();
        let mut counts = vec![
            drops.address_length_drops,
            drops.global_broadcast_dadr_drops,
            drops.broadcast_pdu_type_drops,
            drops.group_dadr_drops,
            claims.reject_applied,
            claims.reject_dampened,
            claims.reject_dampened_same_state,
            claims.reject_dampened_hold_down,
            claims.learned_ok,
            claims.learned_cap_ignored,
            claims.flap_warned,
            claims.pending_started,
            claims.corroborated_applied,
            claims.pending_expired,
            claims.disconnect_removal_ignored,
            claims.hold_evicted,
            self.hosted.sequence.load(Ordering::SeqCst),
            drops.refused,
        ];
        counts.extend(admission(&self.hosted.apdus, &self.hosted.controls));
        counts.extend(decisions(&self.hosted.gate));
        counts
    }

    async fn legacy_routes(&self) -> Vec<RouteView> {
        let table = self.legacy.table.lock().await;
        let mut networks = table.networks();
        networks.sort_unstable();
        networks
            .into_iter()
            .map(|network| {
                let route = table.lookup(network).unwrap();
                RouteView {
                    network,
                    port: route.port_index,
                    direct: route.directly_connected,
                    next_hop: route.next_hop_mac.to_vec(),
                    last_seen: route.last_seen.map(|at| self.nanos(at)),
                    last_used: route.last_used.map(|at| self.nanos(at)),
                    reachability: match route.reachability {
                        legacy::router_table::ReachabilityStatus::Reachable => 0,
                        legacy::router_table::ReachabilityStatus::Busy => 1,
                        legacy::router_table::ReachabilityStatus::Unreachable => 2,
                    },
                    busy_until: route.busy_until.map(|at| self.nanos(at)),
                    flap_count: route.flap_count,
                    last_port_change: route.last_port_change.map(|at| self.nanos(at)),
                    pending: table
                        .pending(network)
                        .map(|(port, at)| (port, self.nanos(at))),
                }
            })
            .collect()
    }

    fn hosted_routes(&self) -> Vec<RouteView> {
        use bacnet_router_core::table::Reachability;
        let engine = self.hosted.shared.engine.lock();
        let table = engine.table();
        table
            .routes()
            .map(|(network, route)| RouteView {
                network,
                port: usize::from(route.port.0),
                direct: route.direct,
                next_hop: route.next_hop.as_slice().to_vec(),
                last_seen: route.last_seen.map(|at| at.as_nanos()),
                last_used: route.last_used.map(|at| at.as_nanos()),
                reachability: match route.reachability {
                    Reachability::Reachable => 0,
                    Reachability::Busy => 1,
                    Reachability::Unreachable => 2,
                },
                busy_until: route.busy_until.map(|at| at.as_nanos()),
                flap_count: route.flap_count,
                last_port_change: route.last_port_change.map(|at| at.as_nanos()),
                pending: table
                    .pending(network)
                    .map(|pending| (usize::from(pending.port.0), pending.first_seen.as_nanos())),
            })
            .collect()
    }

    async fn legacy_searches(&self) -> Vec<(u16, u64)> {
        self.legacy
            .discovery
            .lock()
            .await
            .solicited()
            .into_iter()
            .map(|(network, at)| (network, self.nanos(at)))
            .collect()
    }

    /// Stop both routers' searches, as `stop` does, and compare.
    pub(super) async fn stop(&mut self) {
        self.legacy.discovery.lock().await.cancel();
        self.hosted.shared.engine.lock().cancel_discovery();
        self.clogged.fill(false);
        self.check("stop").await;
    }
}
