use std::net::SocketAddrV6;

use bacnet_transport::bip6::{VMAC_TEXT_PREFIX, parse_vmac_text};
use bacnet_types::error::Error;

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
/// - Hex:  `"01:02:03:04:05:06"` → raw bytes, two or more colon-separated
///   pairs (an SC VMAC, an Ethernet MAC, or a B/IPv6 VMAC such as
///   `"00:04:d2"`)
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
    if address.starts_with(VMAC_TEXT_PREFIX) {
        return parse_vmac_text(address)
            .map(|vmac| vmac.octets().to_vec())
            .map_err(value_error);
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

    // Hex colon notation: two or more colon-separated hex pairs.
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

/// The addresses one `BACnetClient` takes: [`parse_address`], except that a
/// `bip6:` VMAC names a B/IPv6 node, so a client on any other transport
/// refuses it with a `ValueError` that says so, rather than sending a
/// 3-octet MAC its transport can't use.
#[derive(Clone, Copy)]
pub(crate) struct PeerAddresses {
    bip6: bool,
}

impl PeerAddresses {
    /// The addresses a client with this `transport` keyword takes.
    pub(crate) fn for_transport(transport: &str) -> Self {
        Self {
            bip6: transport == "ipv6",
        }
    }

    /// Parse `address` for this client.
    pub(crate) fn parse(self, address: &str) -> PyResult<Vec<u8>> {
        if !self.bip6 && address.starts_with(VMAC_TEXT_PREFIX) {
            return Err(PyValueError::new_err(format!(
                "'{address}' is a B/IPv6 VMAC: bip6: addresses need transport='ipv6'"
            )));
        }
        parse_address(address)
    }
}

/// A `ValueError` carrying a parse error's own message, without the
/// `encoding error:` or `value out of range:` prefix its `Display` adds.
fn value_error(error: Error) -> PyErr {
    match error {
        Error::Encoding(message) | Error::OutOfRange(message) => PyValueError::new_err(message),
        Error::Range(refused) => PyValueError::new_err(refused.message().to_string()),
        other => PyValueError::new_err(other.to_string()),
    }
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

fn parse_bip_address(address: &str, field: &str) -> PyResult<Vec<u8>> {
    let (ip_str, port_str) = address.rsplit_once(':').ok_or_else(|| {
        PyValueError::new_err(format!(
            "{field} must be an IPv4 BACnet/IP address in 'ip:port' form"
        ))
    })?;
    let ip: Ipv4Addr = ip_str.parse().map_err(|_| {
        PyValueError::new_err(format!(
            "{field} must be an IPv4 BACnet/IP address in 'ip:port' form"
        ))
    })?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid {field} port: {e}")))?;
    if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return Err(PyValueError::new_err(format!(
            "{field} must be a unicast IPv4 BACnet/IP address"
        )));
    }
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    Ok(mac)
}

/// A direct BACnet/IP destination with an explicitly preserved UDP port.
#[pyclass(name = "DirectTarget", module = "rusty_bacnet", frozen, from_py_object)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyDirectTarget {
    pub(crate) address: String,
    pub(crate) mac: Vec<u8>,
}

#[pymethods]
impl PyDirectTarget {
    #[new]
    pub(crate) fn new(address: String) -> PyResult<Self> {
        let mac = parse_bip_address(&address, "address")?;
        Ok(Self { address, mac })
    }

    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    #[getter]
    fn mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.mac)
    }

    fn __repr__(&self) -> String {
        format!("DirectTarget(address='{}')", self.address)
    }
}

/// A routed BACnet destination with an explicit next-hop router, DNET, and DADR.
#[pyclass(name = "RoutedTarget", module = "rusty_bacnet", frozen, from_py_object)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyRoutedTarget {
    pub(crate) router: String,
    pub(crate) router_mac: Vec<u8>,
    pub(crate) network: u16,
    pub(crate) address: Vec<u8>,
}

/// A Python target accepted by direct/routed client operations.
#[derive(FromPyObject)]
pub enum PyTarget {
    /// Backward-compatible address string.
    Address(String),
    /// Explicit direct B/IP target.
    Direct(PyDirectTarget),
    /// Explicit routed B/IP target.
    Routed(PyRoutedTarget),
}

type TargetParts = (Vec<u8>, Option<(u16, Vec<u8>)>);

impl PyTarget {
    pub(crate) fn into_parts(self, peers: PeerAddresses) -> PyResult<TargetParts> {
        match self {
            Self::Address(address) => Ok((peers.parse(&address)?, None)),
            Self::Direct(target) => Ok((target.mac, None)),
            Self::Routed(target) => Ok((target.router_mac, Some((target.network, target.address)))),
        }
    }
}

#[pymethods]
impl PyRoutedTarget {
    #[new]
    pub(crate) fn new(router: String, network: u16, address: Vec<u8>) -> PyResult<Self> {
        let router_mac = parse_bip_address(&router, "router")?;
        if network == 0 || network == u16::MAX {
            return Err(PyValueError::new_err(
                "network must be a remote DNET in the range 1..=65534",
            ));
        }
        if address.is_empty() || address.len() > bacnet_npdu::MAX_MAC_LEN {
            return Err(PyValueError::new_err(
                "address must contain between 1 and 18 DADR bytes",
            ));
        }
        Ok(Self {
            router,
            router_mac,
            network,
            address,
        })
    }

    #[getter]
    fn router(&self) -> &str {
        &self.router
    }

    #[getter]
    fn router_mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.router_mac)
    }

    #[getter]
    fn network(&self) -> u16 {
        self.network
    }

    #[getter]
    fn address<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.address)
    }

    fn __repr__(&self) -> String {
        format!(
            "RoutedTarget(router='{}', network={}, address={:?})",
            self.router, self.network, self.address
        )
    }
}
