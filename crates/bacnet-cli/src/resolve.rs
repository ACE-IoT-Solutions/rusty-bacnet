use std::any::Any;
use std::net::{Ipv4Addr, SocketAddrV6};

use bacnet_transport::bip6::{vmac_from_device_instance, Bip6Transport, DEFAULT_BACNET6_PORT};
use bacnet_transport::port::TransportPort;

/// A parsed target for a BACnet command.
pub enum Target {
    /// Direct MAC address (already resolved).
    Mac(Vec<u8>),
    /// A B/IPv6 address. The node there is addressed by its VMAC, which
    /// [`bip6_vmac`] finds.
    Bip6(SocketAddrV6),
    /// Device instance number (needs discovery lookup).
    Instance(u32),
    /// Routed device: (dnet, device_instance).
    Routed(u16, u32),
}

/// Encode an IPv4 address and port as a 6-byte BIP MAC address.
///
/// The format is 4 bytes of IPv4 address followed by 2 bytes of port in
/// big-endian byte order.
pub fn ip_to_bip_mac(ip: Ipv4Addr, port: u16) -> Vec<u8> {
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}

/// Parse a target string into a [`Target`].
///
/// Supported formats:
/// - `dnet:instance` — routed device (e.g., `2:1234`)
/// - `ip` or `ip:port` — direct BIP MAC (e.g., `192.168.1.100` or `192.168.1.100:47808`)
/// - `bip6:instance` or `bip6:0xHHHHHH` — a B/IPv6 VMAC: a device instance's
///   (Clause H.7.2), or any three octets, as [`format_bip6_vmac`] prints them
/// - `[ipv6]` or `[ipv6]:port` — a B/IPv6 address (port 47808 when absent),
///   whose VMAC [`bip6_vmac`] finds
/// - `instance` — device instance number (e.g., `1234`)
pub fn parse_target(s: &str) -> Result<Target, String> {
    if let Some(vmac) = s.strip_prefix("bip6:") {
        return parse_bip6_vmac(vmac).map(|vmac| Target::Mac(vmac.to_vec()));
    }
    if s.starts_with('[') {
        return parse_bip6_address(s).map(Target::Bip6);
    }

    // Try parsing as dnet:instance (both parts must be numeric).
    if let Some((left, right)) = s.split_once(':') {
        if let (Ok(dnet), Ok(instance)) = (left.parse::<u16>(), right.parse::<u32>()) {
            return Ok(Target::Routed(dnet, instance));
        }
        // If only the left side fails as u16, it might be an IP:port.
    }

    // Try parsing as IP address with optional port.
    if let Some((ip_str, port_str)) = s.split_once(':') {
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            let port = port_str
                .parse::<u16>()
                .map_err(|_| format!("invalid port in target '{s}'"))?;
            return Ok(Target::Mac(ip_to_bip_mac(ip, port)));
        }
    }

    // Try parsing as bare IP address (no port).
    if let Ok(ip) = s.parse::<Ipv4Addr>() {
        return Ok(Target::Mac(ip_to_bip_mac(ip, 0xBAC0)));
    }

    // Try parsing as plain device instance number.
    if let Ok(instance) = s.parse::<u32>() {
        return Ok(Target::Instance(instance));
    }

    Err(format!(
        "invalid target '{s}': expected IP[:port], bip6:INSTANCE, bip6:0xVMAC, \
         [IPv6][:port], device instance, or dnet:instance"
    ))
}

/// The VMAC a `bip6:` target names, after the prefix: a decimal device
/// instance, whose VMAC is its 22 bits (Clause H.7.2), or `0x` and six hex
/// digits for any VMAC, such as a client's random one.
fn parse_bip6_vmac(text: &str) -> Result<[u8; 3], String> {
    let form = || {
        format!("invalid B/IPv6 VMAC 'bip6:{text}': expected bip6:INSTANCE (0-4194303) or bip6:0x and 6 hex digits")
    };
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(form());
        }
        let value = u32::from_str_radix(hex, 16).map_err(|_| form())?;
        let [_, high, middle, low] = value.to_be_bytes();
        return Ok([high, middle, low]);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(form());
    }
    // Digits past u32 are out of range like any instance above 4194303.
    let instance = text.parse::<u32>().unwrap_or(u32::MAX);
    vmac_from_device_instance(instance)
        .map(|vmac| vmac.octets())
        .map_err(|_| format!("B/IPv6 device instance {text} is out of range (0-4194303)"))
}

/// `[ipv6]` or `[ipv6]:port`, port 47808 when absent. A link-local address
/// may carry its interface as a numeric zone, `[fe80::1%3]`.
fn parse_bip6_address(s: &str) -> Result<SocketAddrV6, String> {
    let with_port;
    let text = if s.ends_with(']') {
        with_port = format!("{s}:{DEFAULT_BACNET6_PORT}");
        &with_port
    } else {
        s
    };
    text.parse()
        .map_err(|_| format!("invalid B/IPv6 address '{s}': expected [IPv6] or [IPv6]:port"))
}

/// A B/IPv6 VMAC as `bip6:0x0004D2`, the form [`parse_target`] reads back.
pub fn format_bip6_vmac(vmac: [u8; 3]) -> String {
    let [high, middle, low] = vmac;
    format!("bip6:0x{high:02X}{middle:02X}{low:02X}")
}

/// `transport` as the B/IPv6 transport, when it is one. The commands are
/// generic over the transport; only B/IPv6 maps addresses to VMACs.
pub fn as_bip6<T: TransportPort + 'static>(transport: &T) -> Option<&Bip6Transport> {
    (transport as &dyn Any).downcast_ref()
}

/// Find the VMAC of the B/IPv6 node at `peer`, the MAC a command sends to.
///
/// A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
/// U.1.3), so the CLI asks the node once, with a Virtual-Address-Resolution
/// (Clause U.5), unless the transport already knows which VMAC is there.
/// Fails on any transport but B/IPv6, and when nothing answers within three
/// seconds.
pub async fn bip6_vmac<T: TransportPort + 'static>(
    transport: &T,
    peer: SocketAddrV6,
) -> Result<Vec<u8>, String> {
    let bip6 = as_bip6(transport)
        .ok_or_else(|| format!("{peer} is a B/IPv6 address: use it with --ipv6"))?;
    let vmac = bip6
        .resolve_virtual_address(peer)
        .await
        .map_err(|e| format!("could not find the VMAC at {peer}: {e}"))?;
    Ok(vmac.octets().to_vec())
}

/// Why a directed Who-Is refuses a device instance or routed target.
pub const TARGET_NOT_AN_ADDRESS: &str =
    "--target requires an address, not a device instance or routed address";

/// The MAC of a target that must be an address, such as a directed
/// Who-Is's: a MAC as written, or the VMAC [`bip6_vmac`] finds for a
/// B/IPv6 address. A device instance or routed target fails with `refusal`.
pub async fn address_mac<T: TransportPort + 'static>(
    transport: &T,
    text: &str,
    refusal: &str,
) -> Result<Vec<u8>, String> {
    match parse_target(text)? {
        Target::Mac(mac) => Ok(mac),
        Target::Bip6(peer) => bip6_vmac(transport, peer).await,
        Target::Instance(_) | Target::Routed(..) => Err(refusal.to_string()),
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
