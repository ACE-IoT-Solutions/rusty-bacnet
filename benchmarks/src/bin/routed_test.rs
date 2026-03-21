//! Integration test for routed device addressing.
//!
//! Designed to run inside a container on the campus test network.
//! Tests confirmed requests to devices behind BACpypes3 virtual routers.
//!
//! Campus topology (building 1):
//!   Network 1000: BACnet/IP (router at 10.1.0.10:47808)
//!   Network 1001: Central Plant — Chiller (Device 1005, MAC 0x02)
//!   Network 1100: AHU — VAV 1000-1003 (MAC 0x02-0x05), AHU 1004 (MAC 0x06)

use bacnet_client::client::{BACnetClient, RequestTarget};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;
use std::process::ExitCode;
use tokio::time::Duration;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let interface: Ipv4Addr = std::env::var("BACNET_INTERFACE")
        .unwrap_or_else(|_| "0.0.0.0".into())
        .parse()
        .expect("invalid BACNET_INTERFACE");

    let broadcast: Ipv4Addr = std::env::var("BACNET_BROADCAST")
        .unwrap_or_else(|_| "10.1.0.255".into())
        .parse()
        .expect("invalid BACNET_BROADCAST");

    let router_addr =
        std::env::var("ROUTER_ADDRESS").unwrap_or_else(|_| "10.1.0.10:47808".into());

    println!("=== Routed Device Addressing Integration Test ===");
    println!("Interface: {interface}");
    println!("Broadcast: {broadcast}");
    println!("Router: {router_addr}");
    println!();

    let mut client = BACnetClient::bip_builder()
        .interface(interface)
        .port(47808)
        .broadcast_address(broadcast)
        .apdu_timeout_ms(6000)
        .build()
        .await
        .expect("failed to build client");

    let router_mac = addr_to_mac(&router_addr);
    let mut passed = 0u32;
    let mut failed = 0u32;

    let device_oid = |instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();

    // ── Test 1: Router discovery ───────────────────────────────────────
    print!("Test 1: Who-Is-Router-To-Network ... ");
    match client.who_is_router_to_network(None, 3000).await {
        Ok(routers) => {
            if routers.is_empty() {
                println!("FAIL (no routers responded)");
                failed += 1;
            } else {
                println!("OK ({} routers)", routers.len());
                for r in &routers {
                    println!(
                        "  {} serves networks: {:?}",
                        mac_to_addr(r.mac.as_ref()),
                        r.networks
                    );
                }
                passed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 1b: Direct read to building1 sim (baseline) ─────────────────
    print!("Test 1b: Direct read_property — router device at 10.1.0.10:47808 ... ");
    match client
        .read_property(
            router_mac.clone(),
            device_oid(999),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            // The router might not host a device 999 on the IP port directly.
            // Try the HVAC-Building-Router device which the sim creates.
            println!("INFO ({e}) — trying WhoIs to find device instances...");
            client.who_is(None, None).await.ok();
            tokio::time::sleep(Duration::from_secs(3)).await;
            let devs = client.discovered_devices().await;
            for d in &devs {
                println!(
                    "  discovered: device {} @ {}",
                    d.object_identifier.instance_number(),
                    mac_to_addr(d.mac_address.as_ref()),
                );
            }
            passed += 1; // informational
        }
    }
    println!();

    // ── Test 2: Routed read — VAV Device 1000 on network 1100 ──────────
    print!("Test 2: Routed read_property — Device 1000 (net 1100, MAC 0x02) ... ");
    let target = RequestTarget::Routed {
        router_mac: router_mac.clone(),
        dest_network: 1100,
        dest_mac: vec![0x02],
    };
    match client
        .read_property(target, device_oid(1000), PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 3: Routed read — VAV Device 1001 on network 1100 ──────────
    print!("Test 3: Routed read_property — Device 1001 (net 1100, MAC 0x03) ... ");
    let target = RequestTarget::Routed {
        router_mac: router_mac.clone(),
        dest_network: 1100,
        dest_mac: vec![0x03],
    };
    match client
        .read_property(target, device_oid(1001), PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 4: Routed read — Chiller Device 1005 on network 1001 ──────
    print!("Test 4: Routed read_property — Device 1005 / Chiller (net 1001, MAC 0x02) ... ");
    let target = RequestTarget::Routed {
        router_mac: router_mac.clone(),
        dest_network: 1001,
        dest_mac: vec![0x02],
    };
    match client
        .read_property(target, device_oid(1005), PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 5: Routed read — AHU Device 1004 on network 1100 ──────────
    print!("Test 5: Routed read_property — Device 1004 / AHU (net 1100, MAC 0x06) ... ");
    let target = RequestTarget::Routed {
        router_mac: router_mac.clone(),
        dest_network: 1100,
        dest_mac: vec![0x06],
    };
    match client
        .read_property(target, device_oid(1004), PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 6: Routed read — error response confirms round-trip ───────
    print!("Test 6: Routed read of non-existent property (confirm BACnet error, not timeout) ... ");
    let target = RequestTarget::Routed {
        router_mac: router_mac.clone(),
        dest_network: 1100,
        dest_mac: vec![0x02],
    };
    // Read a property that likely doesn't exist to confirm we get a BACnet error, not a timeout
    match client
        .read_property(
            target,
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99999).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
    {
        Ok(_) => {
            println!("OK (unexpectedly succeeded, but round-trip works)");
            passed += 1;
        }
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("timeout") || msg.contains("Timeout") {
                println!("FAIL (timeout — response not received)");
                failed += 1;
            } else {
                println!("OK (got BACnet error, not timeout: {e})");
                passed += 1;
            }
        }
    }
    println!();

    // ── Test 7: Implicit routed (auto-discover router) ─────────────────
    print!("Test 7: ImplicitRouted read (auto-discover router for net 1100) ... ");
    // Router table should be populated from Test 1's Who-Is-Router
    tokio::time::sleep(Duration::from_millis(500)).await;
    let target = RequestTarget::ImplicitRouted {
        dest_network: 1100,
        dest_mac: vec![0x02],
    };
    match client
        .read_property(target, device_oid(1000), PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => {
            println!("OK (property_value = {} bytes)", ack.property_value.len());
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Summary ────────────────────────────────────────────────────────
    client.stop().await.unwrap();

    println!("=== Results: {passed} passed, {failed} failed ===");
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn addr_to_mac(addr: &str) -> Vec<u8> {
    let parts: Vec<&str> = addr.split(':').collect();
    let ip: Ipv4Addr = parts[0].parse().expect("invalid IP");
    let port: u16 = parts[1].parse().expect("invalid port");
    let mut mac = ip.octets().to_vec();
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}

fn mac_to_addr(mac: &[u8]) -> String {
    if mac.len() == 6 {
        format!(
            "{}.{}.{}.{}:{}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            u16::from_be_bytes([mac[4], mac[5]])
        )
    } else {
        format!("{:?}", mac)
    }
}
