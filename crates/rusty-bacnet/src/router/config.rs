use std::collections::HashSet;
use std::net::Ipv4Addr;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::validate_network_number;

const BROADCAST_VMAC: [u8; 6] = [0xff; 6];

/// Typed BACnet/IP router-port configuration.
#[pyclass(name = "RouterBipPort", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRouterBipPort {
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) interface: String,
    #[pyo3(get)]
    pub(super) port: u16,
    #[pyo3(get)]
    pub(super) broadcast_address: String,
    #[pyo3(get)]
    pub(super) reuse_port: bool,
}

#[pymethods]
impl PyRouterBipPort {
    #[new]
    #[pyo3(signature = (network_number, interface="0.0.0.0", port=0xBAC0, broadcast_address="255.255.255.255", reuse_port=false))]
    fn new(
        network_number: u16,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        reuse_port: bool,
    ) -> PyResult<Self> {
        validate_network_number(network_number)?;
        interface.parse::<Ipv4Addr>().map_err(|error| {
            PyValueError::new_err(format!("invalid interface IPv4 address: {error}"))
        })?;
        broadcast_address.parse::<Ipv4Addr>().map_err(|error| {
            PyValueError::new_err(format!("invalid broadcast IPv4 address: {error}"))
        })?;
        Ok(Self {
            network_number,
            interface: interface.to_owned(),
            port,
            broadcast_address: broadcast_address.to_owned(),
            reuse_port,
        })
    }
}

/// Typed BACnet/SC router-port configuration.
#[pyclass(name = "RouterScPort", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRouterScPort {
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) primary_hub: String,
    #[pyo3(get)]
    pub(super) local_vmac: Vec<u8>,
    #[pyo3(get)]
    pub(super) device_uuid: Vec<u8>,
    #[pyo3(get)]
    pub(super) ca_cert: String,
    #[pyo3(get)]
    pub(super) client_cert: String,
    #[pyo3(get)]
    pub(super) client_key: String,
    #[pyo3(get)]
    pub(super) failover_hub: Option<String>,
    #[pyo3(get)]
    pub(super) heartbeat_interval_ms: u64,
    #[pyo3(get)]
    pub(super) heartbeat_timeout_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_initial_delay_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_max_delay_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_max_retries: u32,
    #[pyo3(get)]
    pub(super) reconnect_forever: bool,
}

#[pymethods]
impl PyRouterScPort {
    #[new]
    #[pyo3(signature = (
        network_number,
        primary_hub,
        local_vmac,
        device_uuid,
        ca_cert,
        client_cert,
        client_key,
        failover_hub=None,
        heartbeat_interval_ms=30000,
        heartbeat_timeout_ms=60000,
        reconnect_initial_delay_ms=10000,
        reconnect_max_delay_ms=600000,
        reconnect_max_retries=10,
        reconnect_forever=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        network_number: u16,
        primary_hub: String,
        local_vmac: Vec<u8>,
        device_uuid: Vec<u8>,
        ca_cert: String,
        client_cert: String,
        client_key: String,
        failover_hub: Option<String>,
        heartbeat_interval_ms: u64,
        heartbeat_timeout_ms: u64,
        reconnect_initial_delay_ms: u64,
        reconnect_max_delay_ms: u64,
        reconnect_max_retries: u32,
        reconnect_forever: bool,
    ) -> PyResult<Self> {
        validate_network_number(network_number)?;
        validate_hub(&primary_hub, "primary_hub")?;
        if let Some(hub) = &failover_hub {
            validate_hub(hub, "failover_hub")?;
            if hub == &primary_hub {
                return Err(PyValueError::new_err(
                    "failover_hub must differ from primary_hub",
                ));
            }
        }
        if local_vmac.len() != 6 {
            return Err(PyValueError::new_err("local_vmac must be exactly 6 bytes"));
        }
        if local_vmac.as_slice() == [0; 6] || local_vmac.as_slice() == BROADCAST_VMAC {
            return Err(PyValueError::new_err(
                "local_vmac must be neither all-zero nor broadcast",
            ));
        }
        if device_uuid.len() != 16 {
            return Err(PyValueError::new_err(
                "device_uuid must be exactly 16 bytes",
            ));
        }
        if device_uuid.iter().all(|byte| *byte == 0) {
            return Err(PyValueError::new_err("device_uuid must not be all-zero"));
        }
        for (value, name) in [
            (&ca_cert, "ca_cert"),
            (&client_cert, "client_cert"),
            (&client_key, "client_key"),
        ] {
            if value.trim().is_empty() {
                return Err(PyValueError::new_err(format!(
                    "{name} must be a nonempty path for SC mutual TLS"
                )));
            }
        }
        if !(3_000..=300_000).contains(&heartbeat_interval_ms) {
            return Err(PyValueError::new_err(
                "heartbeat_interval_ms must be in 3000..=300000",
            ));
        }
        if heartbeat_timeout_ms <= heartbeat_interval_ms {
            return Err(PyValueError::new_err(
                "heartbeat_timeout_ms must be greater than heartbeat_interval_ms",
            ));
        }
        if reconnect_initial_delay_ms == 0 {
            return Err(PyValueError::new_err(
                "reconnect_initial_delay_ms must be greater than zero",
            ));
        }
        if reconnect_max_delay_ms < reconnect_initial_delay_ms {
            return Err(PyValueError::new_err(
                "reconnect_max_delay_ms must be at least reconnect_initial_delay_ms",
            ));
        }
        if reconnect_max_retries == 0 && !reconnect_forever {
            return Err(PyValueError::new_err(
                "reconnect_max_retries must be greater than zero unless reconnect_forever is true",
            ));
        }
        Ok(Self {
            network_number,
            primary_hub,
            local_vmac,
            device_uuid,
            ca_cert,
            client_cert,
            client_key,
            failover_hub,
            heartbeat_interval_ms,
            heartbeat_timeout_ms,
            reconnect_initial_delay_ms,
            reconnect_max_delay_ms,
            reconnect_max_retries,
            reconnect_forever,
        })
    }
}

/// Typed in-process virtual router-port configuration.
#[pyclass(name = "RouterVirtualPort", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRouterVirtualPort {
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) name: String,
    #[pyo3(get)]
    pub(super) mac: u8,
}

#[pymethods]
impl PyRouterVirtualPort {
    #[new]
    fn new(network_number: u16, name: String, mac: u8) -> PyResult<Self> {
        validate_network_number(network_number)?;
        if name.trim().is_empty() {
            return Err(PyValueError::new_err(
                "virtual router network name must not be blank",
            ));
        }
        Ok(Self {
            network_number,
            name,
            mac,
        })
    }
}

#[derive(FromPyObject, Clone)]
pub(super) enum PyRouterPortConfig {
    Bip(PyRouterBipPort),
    Sc(PyRouterScPort),
    Virtual(PyRouterVirtualPort),
}

impl PyRouterPortConfig {
    pub(super) fn network_number(&self) -> u16 {
        match self {
            Self::Bip(port) => port.network_number,
            Self::Sc(port) => port.network_number,
            Self::Virtual(port) => port.network_number,
        }
    }
}

pub(super) fn validate_ports(ports: &[PyRouterPortConfig]) -> PyResult<()> {
    if ports.len() < 2 {
        return Err(PyValueError::new_err(
            "ports must contain at least two router ports",
        ));
    }
    let mut networks = HashSet::new();
    let mut bip_endpoints = HashSet::new();
    let mut virtual_names = HashSet::new();
    for port in ports {
        let network = port.network_number();
        if !networks.insert(network) {
            return Err(PyValueError::new_err(format!(
                "duplicate router network number {network}"
            )));
        }
        match port {
            PyRouterPortConfig::Bip(port) => {
                let interface = port
                    .interface
                    .parse::<Ipv4Addr>()
                    .expect("RouterBipPort validated its interface");
                if !bip_endpoints.insert((interface, port.port)) {
                    return Err(PyValueError::new_err(format!(
                        "duplicate B/IP router endpoint {}:{}",
                        port.interface, port.port
                    )));
                }
            }
            PyRouterPortConfig::Virtual(port) => {
                if !virtual_names.insert(&port.name) {
                    return Err(PyValueError::new_err(format!(
                        "duplicate virtual router network name {:?}",
                        port.name
                    )));
                }
            }
            PyRouterPortConfig::Sc(_) => {}
        }
    }
    Ok(())
}

fn validate_hub(value: &str, name: &str) -> PyResult<()> {
    if value.trim().is_empty() {
        return Err(PyValueError::new_err(format!("{name} must not be blank")));
    }
    if !value.starts_with("wss://") {
        return Err(PyValueError::new_err(format!(
            "{name} must use a wss:// URL"
        )));
    }
    Ok(())
}
