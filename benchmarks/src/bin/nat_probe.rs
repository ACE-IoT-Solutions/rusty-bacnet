//! A foreign device that checks a NAT topology (#940, #942): it registers
//! with a BBMD, broadcasts a Who-Is through it, and waits for an I-Am from
//! every device instance it is told to expect. It exits 0 when all answer
//! in time and 1 otherwise, printing who answered and from where.
//!
//! `examples/docker/nat` runs it against two BBMD sites behind NAT routers.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a check tool reports on the console
#![forbid(unsafe_code)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::process::ExitCode;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::{BipMode, ForeignDeviceConfig};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "nat-probe",
    about = "Who-Is through a BBMD, as a foreign device"
)]
struct Args {
    /// The BBMD to register with, ip:port.
    #[arg(long)]
    bbmd: SocketAddrV4,

    /// Comma-separated device instances that must answer.
    #[arg(long, value_delimiter = ',', required = true)]
    expect: Vec<u32>,

    /// Seconds to wait for every answer.
    #[arg(long, default_value_t = 15)]
    timeout: u64,

    /// Seconds between registrations: short behind a NAT router.
    #[arg(long, default_value_t = 20)]
    renewal: u64,
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: *args.bbmd.ip(),
            bbmd_port: args.bbmd.port(),
            ttl: 300,
            renewal_interval: Some(Duration::from_secs(args.renewal)),
        }))
        .build()
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout);
    let mut missing = args.expect.clone();
    while !missing.is_empty() && tokio::time::Instant::now() < deadline {
        client.who_is(None).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let found = client.discovered_devices().await;
        missing.retain(|instance| {
            !found
                .iter()
                .any(|device| device.object_identifier.instance_number() == *instance)
        });
    }
    for device in client.discovered_devices().await {
        println!(
            "I-Am from device {} at {:02x?}",
            device.object_identifier.instance_number(),
            device.mac_address.as_slice()
        );
    }
    if missing.is_empty() {
        println!("nat-probe: every expected device answered");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("nat-probe: no answer from {missing:?}");
        Ok(ExitCode::FAILURE)
    }
}
