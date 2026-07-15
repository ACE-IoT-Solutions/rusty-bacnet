use super::*;

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// Parse an address string to a MAC byte vector.
///
/// Supported formats:
/// - IPv4: `"192.168.1.100:47808"` → 6-byte MAC (4-byte IP + 2-byte port BE)
/// - IPv6: `"[::1]:47808"` → 18-byte MAC (16-byte IPv6 + 2-byte port BE)
/// - Hex:  `"01:02:03:04:05:06"` → raw bytes (for SC VMAC or Ethernet MAC)
pub fn parse_address(address: &str) -> PyResult<Vec<u8>> {
    // IPv6 bracket notation: [addr]:port
    if address.starts_with('[') {
        let close = address
            .find(']')
            .ok_or_else(|| PyValueError::new_err("IPv6 address missing closing bracket"))?;
        let ip_str = &address[1..close];
        let ip: std::net::Ipv6Addr = ip_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid IPv6 address: {e}")))?;
        let rest = &address[close + 1..];
        let port_str = rest
            .strip_prefix(':')
            .ok_or_else(|| PyValueError::new_err("expected ':port' after IPv6 address"))?;
        let port: u16 = port_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
        let mut mac = Vec::with_capacity(18);
        mac.extend_from_slice(&ip.octets());
        mac.extend_from_slice(&port.to_be_bytes());
        return Ok(mac);
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
        PyValueError::new_err("address must be 'ip:port', '[ipv6]:port', or 'aa:bb:...' hex")
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
#[pyclass(name = "DirectTarget", frozen, from_py_object)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyDirectTarget {
    pub(crate) address: String,
    pub(crate) mac: Vec<u8>,
}

#[pymethods]
impl PyDirectTarget {
    #[new]
    fn new(address: String) -> PyResult<Self> {
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
#[pyclass(name = "RoutedTarget", frozen, from_py_object)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyRoutedTarget {
    pub(crate) router: String,
    pub(crate) router_mac: Vec<u8>,
    pub(crate) network: u16,
    pub(crate) address: Vec<u8>,
}

#[derive(FromPyObject)]
pub enum PyTarget {
    Address(String),
    Direct(PyDirectTarget),
    Routed(PyRoutedTarget),
}

impl PyTarget {
    pub fn into_parts(self) -> PyResult<(Vec<u8>, Option<(u16, Vec<u8>)>)> {
        match self {
            Self::Address(address) => Ok((parse_address(&address)?, None)),
            Self::Direct(target) => Ok((target.mac, None)),
            Self::Routed(target) => Ok((target.router_mac, Some((target.network, target.address)))),
        }
    }
}

#[pymethods]
impl PyRoutedTarget {
    #[new]
    fn new(router: String, network: u16, address: Vec<u8>) -> PyResult<Self> {
        let router_mac = parse_bip_address(&router, "router")?;
        if network == 0 || network == u16::MAX {
            return Err(PyValueError::new_err(
                "network must be a remote DNET in the range 1..=65534",
            ));
        }
        if address.is_empty() || address.len() > u8::MAX as usize {
            return Err(PyValueError::new_err(
                "address must contain between 1 and 255 DADR bytes",
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
