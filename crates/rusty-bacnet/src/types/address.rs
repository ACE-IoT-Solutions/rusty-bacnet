use std::net::SocketAddrV6;

use bacnet_transport::bip6::vmac_from_device_instance;

use super::*;

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// Parse an address string to a MAC byte vector.
///
/// Supported formats:
/// - IPv4: `"192.168.1.100:47808"` → 6-byte MAC (4-byte IP + 2-byte port BE)
/// - B/IPv6: `"bip6:1234"` → the 3-byte VMAC of device instance 1234
///   (0..=4194303, Clause H.7.2), or `"bip6:0x0004D2"` → any 3-byte VMAC. A
///   B/IPv6 node is addressed by its VMAC (Clause U.1.3).
/// - Hex:  `"01:02:03:04:05:06"` → raw bytes (an SC VMAC, an Ethernet MAC, or
///   a B/IPv6 VMAC such as `"00:04:d2"`)
/// - MS/TP: `"7"` or `"mstp:7"` → 1-byte unicast peer MAC (0..=254)
///
/// A bracketed IPv6 address such as `"[::1]:47808"` raises `ValueError`: it
/// is where a node is, not its MAC. `BACnetClient.resolve_vmac` finds the
/// VMAC of the node at such an address.
pub fn parse_address(address: &str) -> PyResult<Vec<u8>> {
    // MS/TP unicast peer: "mstp:N" or bare decimal 0..=254.
    if let Some(rest) = address.strip_prefix("mstp:") {
        return parse_mstp_peer_address(rest);
    }
    if let Some(rest) = address.strip_prefix("bip6:") {
        return parse_bip6_vmac(rest).map(Vec::from);
    }
    let is_negative_decimal = address
        .strip_prefix('-')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
    if address.bytes().all(|b| b.is_ascii_digit()) || is_negative_decimal {
        return parse_mstp_peer_address(address);
    }

    if address.starts_with('[') {
        return Err(PyValueError::new_err(format!(
            "'{address}' is a B/IPv6 address, not a MAC: name the node by its VMAC, \
             'bip6:<device instance>' or 'bip6:0x<6 hex digits>', or find the VMAC \
             with 'await client.resolve_vmac(\"[ip]:port\")'"
        )));
    }

    // Hex colon notation: aa:bb:cc:dd:ee:ff (6 or more hex pairs)
    if address.contains(':')
        && address
            .split(':')
            .all(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let bytes: Result<Vec<u8>, _> = address
            .split(':')
            .map(|s| u8::from_str_radix(s, 16))
            .collect();
        return bytes.map_err(|e| PyValueError::new_err(format!("invalid hex address: {e}")));
    }

    // IPv4: ip:port
    let (ip_str, port_str) = address.rsplit_once(':').ok_or_else(|| {
        PyValueError::new_err(
            "address must be 'ip:port', 'bip6:<device instance>', 'bip6:0x<6 hex digits>', \
             'aa:bb:...' hex, or MS/TP 'N' / 'mstp:N'",
        )
    })?;
    let ip: Ipv4Addr = ip_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid IP address: {e}")))?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    Ok(mac)
}

/// What a `bip6:` address names, after the prefix: a decimal device
/// instance, whose VMAC is its 22 bits (Clause H.7.2), or `0x` and six hex
/// digits for any VMAC, such as a random one.
fn parse_bip6_vmac(text: &str) -> PyResult<[u8; 3]> {
    const FORM: &str =
        "B/IPv6 address must be 'bip6:<device instance 0..=4194303>' or 'bip6:0x<6 hex digits>'";
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(PyValueError::new_err(FORM));
        }
        let value = u32::from_str_radix(hex, 16).map_err(|_| PyValueError::new_err(FORM))?;
        let [_, high, middle, low] = value.to_be_bytes();
        return Ok([high, middle, low]);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PyValueError::new_err(FORM));
    }
    // Digits past u32 are out of range like any instance above 4194303.
    let instance = text.parse::<u32>().unwrap_or(u32::MAX);
    vmac_from_device_instance(instance)
        .map(|vmac| vmac.octets())
        .map_err(|_| PyValueError::new_err("B/IPv6 device instance must be in 0..=4194303"))
}

/// A B/IPv6 VMAC as `bip6:0x0004D2`, the form [`parse_address`] reads back.
pub(crate) fn format_bip6_vmac(vmac: [u8; 3]) -> String {
    let [high, middle, low] = vmac;
    format!("bip6:0x{high:02X}{middle:02X}{low:02X}")
}

/// The B/IPv6 address `resolve_vmac` takes: `"[ip]:port"`, where a
/// link-local address may carry its interface as a numeric zone
/// (`"[fe80::1%3]:47808"`).
pub(crate) fn parse_bip6_address(address: &str) -> PyResult<SocketAddrV6> {
    if !address.starts_with('[') {
        return Err(PyValueError::new_err(format!(
            "'{address}' is not a B/IPv6 address: resolve_vmac takes '[ip]:port'"
        )));
    }
    address.parse().map_err(|_| {
        PyValueError::new_err(format!(
            "invalid B/IPv6 address '{address}': expected '[ip]:port'"
        ))
    })
}

fn parse_mstp_peer_address(address: &str) -> PyResult<Vec<u8>> {
    if address.starts_with('-') {
        return Err(PyValueError::new_err(
            "MS/TP peer address must be in 0..=254",
        ));
    }
    if address.is_empty() || !address.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PyValueError::new_err(
            "MS/TP peer address must be a decimal integer in 0..=254",
        ));
    }
    let mac: u32 = address
        .parse()
        .map_err(|_| PyValueError::new_err("MS/TP peer address must be in 0..=254"))?;
    match mac {
        0..=254 => Ok(vec![mac as u8]),
        255 => Err(PyValueError::new_err(
            "MS/TP peer address 255 is broadcast, not a unicast peer",
        )),
        _ => Err(PyValueError::new_err(
            "MS/TP peer address must be in 0..=254",
        )),
    }
}
