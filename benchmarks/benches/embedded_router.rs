//! Host-only deterministic admission microbenchmarks; see the evidence report.

#[path = "embedded_router/memory.rs"]
mod memory;

use bacnet_mstp_core::{HeaplessQueues, Instant, MstpCoreConfig, ProfileMasterCore};
use bacnet_router_adapters::mstp::{MstpEgress, MstpPeerCapability};
use bacnet_router_core::*;
use criterion::{BenchmarkId, Criterion};
use std::{hint::black_box, time::Duration};

fn network(value: u16) -> NetworkNumber {
    NetworkNumber::new(value).unwrap()
}

fn address(value: u8) -> LinkAddress {
    LinkAddress::try_from_slice(&[value]).unwrap()
}

// Own each accepted packet, consuming the preceding packet synchronously.
// This measures rewrite + one ownership copy, without a scheduler or IO.
struct CopySink([u8; 1497]);

impl Egress for CopySink {
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome {
        self.0[..request.npdu.len()].copy_from_slice(request.npdu);
        black_box(&self.0[..request.npdu.len()]);
        AdmissionOutcome::AcceptedOwned
    }
}

fn forwarding<const ROUTES: usize>(c: &mut Criterion) {
    let mut core = RouterCore::<ROUTES, ROUTES>::new();
    for i in 0..ROUTES {
        core.routes_mut()
            .add_direct(network(100 + i as u16), PortId(1 + i as u8))
            .unwrap();
        assert_eq!(
            core.routes_mut().learn(
                network(1000 + i as u16),
                PortId(ROUTES as u8),
                address(7),
                0,
            ),
            LearnOutcome::Inserted
        );
    }
    let capabilities = PortCapabilities {
        id: PortId(ROUTES as u8),
        network: network(99 + ROUTES as u16),
        kind: PortKind::MstpExtended,
        max_npdu_len: 1497,
    };
    let mut scratch = [0; 1497];
    let mut sink = CopySink([0; 1497]);
    let mut group = c.benchmark_group(format!("forward/routes_{ROUTES}"));
    for learned in [false, true] {
        for length in [64, 497] {
            let destination = if learned { 999 } else { 99 } + ROUTES as u16;
            let [hi, lo] = destination.to_be_bytes();
            let mut npdu = vec![0x5a; length];
            npdu[..7].copy_from_slice(&[1, 0x20, hi, lo, 1, 5, 255]);
            let request = DirectForwardRequest {
                ingress_network: network(10),
                logical_source: LinkAddress::try_from_slice(&[192, 0, 2, 1, 0xba, 0xc0]).unwrap(),
                npdu: &npdu,
                egress_capabilities: capabilities,
                now: 1,
            };
            let expected_length = length + if learned { 9 } else { 4 };
            assert_eq!(
                core.forward_unicast(request, &mut scratch, &mut sink)
                    .unwrap(),
                ForwardOutcome::Forwarded {
                    final_npdu_len: expected_length
                }
            );
            let kind = if learned {
                "learned_last"
            } else {
                "direct_last"
            };
            group.bench_with_input(BenchmarkId::new(kind, length), &request, |b, request| {
                b.iter(|| {
                    black_box(
                        core.forward_unicast(
                            black_box(*request),
                            black_box(&mut scratch),
                            black_box(&mut sink),
                        )
                        .unwrap(),
                    );
                });
            });
        }
    }
    group.finish();
}

fn duplicates<const CACHE: usize>(c: &mut Criterion) {
    let key = |value| DuplicateKey::new(10, address(1), 200, address(2), value);
    let mut cache = DuplicateCache::<CACHE>::new(100);
    for i in 0..CACHE {
        assert_eq!(cache.observe(key(i as u64), 0), DuplicateOutcome::Inserted);
    }
    let last = key(CACHE as u64 - 1);
    assert_eq!(cache.observe(last, 1), DuplicateOutcome::Duplicate);
    let mut group = c.benchmark_group(format!("duplicate/cache_{CACHE}"));
    group.bench_function("hit_last", |b| {
        b.iter(|| black_box(cache.observe(black_box(last), black_box(1))));
    });
    let mut next = CACHE as u64;
    assert_eq!(cache.observe(key(next), 1), DuplicateOutcome::Replaced);
    group.bench_function("miss_replace", |b| {
        b.iter(|| {
            next += 1;
            black_box(cache.observe(black_box(key(next)), black_box(1)))
        });
    });
    assert_eq!(cache.len(), CACHE);
    group.finish();
}

fn broadcast<const CACHE: usize, const PORTS: usize>(c: &mut Criterion) {
    let mut policy = BroadcastPolicy::<CACHE>::new(BroadcastPolicyConfig {
        allow_remote: true,
        allow_global: true,
        echo_retention: 10,
    });
    let targets = std::array::from_fn::<_, PORTS, _>(|i| BroadcastTarget {
        capabilities: PortCapabilities {
            id: PortId(1 + i as u8),
            network: network(100 + i as u16),
            kind: PortKind::MstpStandard,
            max_npdu_len: 501,
        },
    });
    let mut results = [BroadcastFanoutResult {
        port: PortId(0),
        outcome: BroadcastFanoutOutcome::PolicyDisabled,
    }; PORTS];
    let mut npdu = [0x5a; 64];
    npdu[..6].copy_from_slice(&[1, 0x20, 255, 255, 0, 255]);
    let mut request = BroadcastRequest {
        ingress_port: PortId(0),
        origin_network: network(10),
        origin_address: address(1),
        destination: BroadcastDestination::Global,
        npdu: &npdu,
        hop_count_offset: Some(5),
        packet_fingerprint: 42,
        class: TrafficClass::Data,
        priority: 0,
        now: 0,
    };
    let mut scratch = [0; 501];
    let mut sink = CopySink([0; 1497]);
    let report = policy
        .forward(request, &targets, &mut results, &mut scratch, &mut sink)
        .unwrap();
    assert_eq!(report.accepted, PORTS);
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, BroadcastFanoutOutcome::Accepted { .. })));
    c.bench_function(&format!("broadcast/cache_{CACHE}/fanout_{PORTS}"), |b| {
        b.iter(|| {
            // Each offered packet arrives at the preceding entry's expiry.
            // The timed path includes purge, insertion, rewrite, and fanout.
            request.now += 10;
            black_box(
                policy
                    .forward(
                        black_box(request),
                        &targets,
                        &mut results,
                        &mut scratch,
                        &mut sink,
                    )
                    .unwrap(),
            );
        });
    });
}

fn saturated<const QUEUE: usize, const NPDU: usize, const FRAME: usize>(c: &mut Criterion) {
    let mut rx = [0; 2048];
    let config = MstpCoreConfig {
        station: 1,
        max_master: 2,
        ..Default::default()
    };
    let queues = HeaplessQueues::<QUEUE, QUEUE, NPDU, 2>::new();
    let mut master =
        ProfileMasterCore::<_, NPDU, FRAME>::new(config, queues, &mut rx, Instant::ZERO).unwrap();
    let mut adapter = MstpEgress::new(&mut master, PortId(2), Instant::ZERO);
    if NPDU > 501 {
        adapter.set_peer_capability(MstpPeerCapability::Extended);
    }
    let mut npdu = [0x5a; NPDU];
    npdu[..2].copy_from_slice(&[1, 0]);
    let request = EgressRequest {
        port: PortId(2),
        destination: EgressDestination::Unicast(address(2)),
        class: TrafficClass::Data,
        priority: 0,
        npdu: &npdu,
    };
    for _ in 0..QUEUE {
        assert_eq!(adapter.try_admit(request), AdmissionOutcome::AcceptedOwned);
    }
    assert_eq!(adapter.try_admit(request), AdmissionOutcome::Full);
    // A saturated data partition must still admit reserved network control.
    assert_eq!(
        adapter.try_admit(EgressRequest {
            class: TrafficClass::NetworkControl,
            npdu: &[1, 0x80, 0, 0, 200],
            ..request
        }),
        AdmissionOutcome::AcceptedOwned
    );
    c.bench_function(&format!("saturated/npdu_{NPDU}/queue_{QUEUE}"), |b| {
        b.iter(|| black_box(adapter.try_admit(black_box(request))));
    });
    assert_eq!(adapter.core_mut().counters().unicast.depth as usize, QUEUE);
}

fn main() {
    memory::print();
    if std::env::var_os("ROUTER_BENCH_MEMORY_ONLY").is_some() {
        return;
    }
    let mut c = Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(1))
        .configure_from_args();
    forwarding::<2>(&mut c);
    forwarding::<8>(&mut c);
    forwarding::<32>(&mut c);
    duplicates::<8>(&mut c);
    duplicates::<32>(&mut c);
    duplicates::<128>(&mut c);
    broadcast::<8, 2>(&mut c);
    broadcast::<32, 4>(&mut c);
    broadcast::<128, 8>(&mut c);
    saturated::<1, 501, 511>(&mut c);
    saturated::<4, 501, 511>(&mut c);
    saturated::<8, 501, 511>(&mut c);
    saturated::<1, 1497, 1516>(&mut c);
    saturated::<4, 1497, 1516>(&mut c);
    saturated::<8, 1497, 1516>(&mut c);
    c.final_summary();
}
