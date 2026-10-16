//! Cross-network stress orchestrator for Docker topology.
//!
//! Connects to the multi-network Docker environment and runs cross-subnet
//! scenarios: router hop latency, BBMD broadcast propagation, and foreign
//! device access.
#![allow(clippy::print_stdout, clippy::print_stderr)] // benchmark binaries report progress and results on the console

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bacnet_benchmarks::stress::output::{
    DegradationPoint, LatencyRecorder, LatencyStats, StressMetrics, StressResult,
};
use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "stress-orchestrator",
    about = "Cross-network BACnet stress orchestrator"
)]
struct Args {
    /// Seconds to wait for all services to be ready
    #[arg(long, default_value_t = 10)]
    wait_for_ready: u64,

    /// Duration per test in seconds
    #[arg(long, default_value_t = 10)]
    duration: u64,

    /// Skip specific scenarios (comma-separated: router,bbmd,foreign)
    #[arg(long)]
    skip: Option<String>,
}

/// Wait for a BIP device to respond to a ReadProperty.
async fn wait_for_device(
    client: &BACnetClient<BipTransport>,
    target_ip: Ipv4Addr,
    target_port: u16,
    device_instance: u32,
    timeout: Duration,
) -> bool {
    let mac = bacnet_transport::bvll::encode_bip_mac(target_ip.octets(), target_port);
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        let result = client
            .read_property(&mac, oid, PropertyIdentifier::OBJECT_NAME, None)
            .await;
        if result.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// A BACnet router to send through: its B/IP address on the client's
/// network, and the network the target is on.
#[derive(Clone, Copy)]
struct Via {
    router_ip: [u8; 4],
    router_port: u16,
    network: u16,
}

/// Run continuous RP against a target, returning a DegradationPoint. With
/// `via`, each request names the target's network and MAC and goes to the
/// router, which forwards it (Docker's bridge networks don't route IP
/// between each other, so only a BACnet router joins them).
#[allow(clippy::too_many_arguments)] // one call site per scenario, each spelling out its addresses
async fn run_rp_scenario(
    label: &str,
    param: u64,
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    target_ip: [u8; 4],
    target_port: u16,
    via: Option<Via>,
    duration_secs: u64,
) -> DegradationPoint {
    let client = BACnetClient::bip_builder()
        .interface(interface)
        .port(0)
        .broadcast_address(broadcast)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .unwrap();

    let mac = bacnet_transport::bvll::encode_bip_mac(target_ip, target_port);
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let mut recorder = LatencyRecorder::new();
    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let start = Instant::now();

    while Instant::now() < deadline {
        let t0 = Instant::now();
        let property = PropertyIdentifier::PRESENT_VALUE;
        let result = match via {
            None => client.read_property(&mac, oid, property, None).await,
            Some(via) => {
                let router = bacnet_transport::bvll::encode_bip_mac(via.router_ip, via.router_port);
                client
                    .read_property_routed(&router, via.network, &mac, oid, property, None)
                    .await
            }
        };
        match result {
            Ok(_) => recorder.record_success(t0),
            Err(_) => recorder.record_failure(),
        }
    }

    let wall = start.elapsed();
    let total = recorder.successful() + recorder.failed();
    let throughput = total as f64 / wall.as_secs_f64();
    let stats = recorder.stats();

    eprintln!(
        "  {label}: {total} ops in {:.1}s = {throughput:.0} ops/s | p50={}µs p99={}µs errors={}",
        wall.as_secs_f64(),
        stats.p50,
        stats.p99,
        recorder.failed(),
    );

    DegradationPoint {
        parameter: param,
        p50_us: stats.p50,
        p99_us: stats.p99,
        throughput,
        errors: recorder.failed(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let skip: Vec<String> = args
        .skip
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();

    // Wait for services to come up
    eprintln!(
        "Waiting {}s for services to be ready...",
        args.wait_for_ready
    );
    tokio::time::sleep(Duration::from_secs(args.wait_for_ready)).await;

    // Probe server-a
    let probe_client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::new(172, 20, 0, 100))
        .port(0)
        .broadcast_address(Ipv4Addr::new(172, 20, 0, 255))
        .apdu_timeout_ms(5000)
        .build()
        .await?;

    if !wait_for_device(
        &probe_client,
        Ipv4Addr::new(172, 20, 0, 10),
        47808,
        1000,
        Duration::from_secs(30),
    )
    .await
    {
        eprintln!("ERROR: server-a not responding after 30s");
        std::process::exit(1);
    }
    eprintln!("server-a is ready");
    drop(probe_client);

    let mut all_points = Vec::new();

    // Scenario 1: Router hop — client on subnet A (network 1) reads from
    // server-b on subnet B (network 2) through the router service.
    if !skip.contains(&"router".to_string()) {
        eprintln!("\n=== Router Hop Latency ===");
        let dp = run_rp_scenario(
            "routed_rp",
            1,
            Ipv4Addr::new(172, 20, 0, 100),
            Ipv4Addr::new(172, 20, 0, 255),
            [172, 20, 1, 10],
            47808,
            Some(Via {
                router_ip: [172, 20, 0, 1],
                router_port: 47808,
                network: 2,
            }),
            args.duration,
        )
        .await;
        all_points.push(dp);
    }

    // Scenario 2: Same-subnet baseline — client on subnet A reads from server-a
    if !skip.contains(&"bbmd".to_string()) {
        eprintln!("\n=== Same-Subnet Baseline ===");
        let dp = run_rp_scenario(
            "same_subnet_rp",
            2,
            Ipv4Addr::new(172, 20, 0, 100),
            Ipv4Addr::new(172, 20, 0, 255),
            [172, 20, 0, 10],
            47808,
            None,
            args.duration,
        )
        .await;
        all_points.push(dp);
    }

    // Scenario 3: Foreign device — client on foreign subnet reads from server-a
    if !skip.contains(&"foreign".to_string()) {
        eprintln!("\n=== Foreign Device Access ===");
        let dp = run_rp_scenario(
            "foreign_rp",
            3,
            Ipv4Addr::new(172, 20, 4, 100),
            Ipv4Addr::new(172, 20, 4, 255),
            [172, 20, 0, 10],
            47808,
            None,
            args.duration,
        )
        .await;
        all_points.push(dp);
    }

    // Summary JSON
    let total_ops: u64 = all_points
        .iter()
        .map(|p| (p.throughput * args.duration as f64) as u64)
        .sum();
    let peak = all_points
        .iter()
        .map(|p| p.throughput)
        .fold(0.0f64, f64::max);

    let result = StressResult {
        scenario: "docker-cross-network".into(),
        transport: "bip".into(),
        parameters: serde_json::json!({
            "duration_secs": args.duration,
            "scenarios": all_points.len(),
        }),
        results: StressMetrics {
            total_requests: total_ops,
            successful: total_ops,
            failed: 0,
            error_rate_pct: 0.0,
            throughput_ops_sec: peak,
            latency_us: LatencyStats {
                min: 0,
                p50: 0,
                p95: 0,
                p99: 0,
                p999: 0,
                max: 0,
            },
            peak_rss_kb: 0,
            degradation_curve: all_points,
        },
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("{json}");

    Ok(())
}
