//! Python-composable BACnet/IP and BACnet/SC router.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bacnet_network::router::{
    BACnetRouter as NativeRouter, RouterOptions, RouterPort, StartedRouter,
};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::sc::{ScConnectionState, ScReconnectConfig, ScTransport};
use bacnet_transport::sc_tls::TlsWebSocket;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::errors::to_py_err;

mod config;
mod snapshots;

pub use config::{PyRouterBipPort, PyRouterScPort};
use config::{PyRouterPortConfig, validate_ports};
pub use snapshots::{PyRouterPortHealth, PyRouterRouteEntry};

type MixedTransport = AnyTransport<crate::mstp_py::PySerial>;
const SC_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
enum HealthSource {
    Bip {
        identity: String,
    },
    Sc {
        identity: String,
        state: watch::Receiver<ScConnectionState>,
    },
}

impl HealthSource {
    fn snapshot(&self, config_index: usize, network_number: u16) -> PyRouterPortHealth {
        match self {
            Self::Bip { identity } => {
                snapshots::port_health(config_index, network_number, "bip", identity, None)
            }
            Self::Sc { identity, state } => snapshots::port_health(
                config_index,
                network_number,
                "sc",
                identity,
                Some(*state.borrow()),
            ),
        }
    }
}

struct BuiltPort {
    port: RouterPort<MixedTransport>,
    health: HealthSource,
}

struct RunningRouter {
    router: Arc<Mutex<NativeRouter>>,
    background: Vec<JoinHandle<()>>,
    health: Vec<(u16, HealthSource)>,
}

impl Drop for RunningRouter {
    fn drop(&mut self) {
        for task in &self.background {
            task.abort();
        }
    }
}

/// A mixed BACnet/IP and BACnet/SC router.
#[pyclass(name = "BACnetRouter", module = "rusty_bacnet")]
pub struct PyBACnetRouter {
    inner: Arc<Mutex<Option<RunningRouter>>>,
    lifecycle: Arc<Mutex<()>>,
    configured_ports: Vec<PyRouterPortConfig>,
    announce_interval: Option<Duration>,
    last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
    last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
}

#[pymethods]
impl PyBACnetRouter {
    /// Build a mixed-transport router from typed port configurations.
    #[classmethod]
    #[pyo3(signature = (ports, *, announce_interval_s=None))]
    fn from_ports(
        _class: &Bound<'_, pyo3::types::PyType>,
        ports: Vec<PyRouterPortConfig>,
        announce_interval_s: Option<f64>,
    ) -> PyResult<Self> {
        validate_ports(&ports)?;
        let announce_interval = announce_interval_s
            .map(|seconds| {
                Duration::try_from_secs_f64(seconds).map_err(|_| {
                    PyValueError::new_err(
                        "announce_interval_s must be a finite number greater than zero",
                    )
                })
            })
            .transpose()?;
        if announce_interval.is_some_and(|interval| interval < Duration::from_secs(1)) {
            return Err(PyValueError::new_err(
                "announce_interval_s must be a finite number of at least 1 second",
            ));
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            lifecycle: Arc::new(Mutex::new(())),
            configured_ports: ports,
            announce_interval,
            last_health: Arc::new(StdMutex::new(Vec::new())),
            last_routes: Arc::new(StdMutex::new(Vec::new())),
        })
    }

    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let lifecycle = Arc::clone(&self.lifecycle);
        let configured = self.configured_ports.clone();
        let announce_interval = self.announce_interval;
        crate::py_async::future_into_py(
            py,
            crate::unit_result(async move {
                let _lifecycle = lifecycle.lock().await;
                if inner.lock().await.is_some() {
                    return Err(PyRuntimeError::new_err(
                        "router is already starting or running",
                    ));
                }
                let mut built = Vec::with_capacity(configured.len());
                for (index, port) in configured.into_iter().enumerate() {
                    built.push(build_router_port(index, port).await?);
                }
                let mut ports = Vec::with_capacity(built.len());
                let mut health = Vec::with_capacity(built.len());
                for built in built {
                    health.push((built.port.network_number, built.health));
                    ports.push(built.port);
                }
                let StartedRouter { router, .. } = NativeRouter::start(ports, RouterOptions::new())
                    .await
                    .map_err(to_py_err)?;
                for (index, (_, source)) in health.iter_mut().enumerate() {
                    if let HealthSource::Bip { identity } = source {
                        if let Some(endpoint) = bip_endpoint(&router, index) {
                            *identity = endpoint.to_string();
                        }
                    }
                }
                let router = Arc::new(Mutex::new(router));
                let background = spawn_background(&router, &health, announce_interval);
                let active = RunningRouter {
                    router,
                    background,
                    health,
                };
                *inner.lock().await = Some(active);
                Ok(())
            }),
        )
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let lifecycle = Arc::clone(&self.lifecycle);
        let last_health = Arc::clone(&self.last_health);
        let last_routes = Arc::clone(&self.last_routes);
        crate::py_async::future_into_py(
            py,
            crate::unit_result(async move {
                let _lifecycle = lifecycle.lock().await;
                let mut inner = inner.lock().await;
                let Some(active) = inner.as_mut() else {
                    return Ok(());
                };
                for task in &active.background {
                    task.abort();
                }
                for task in active.background.drain(..) {
                    let _ = task.await;
                }
                let health = health_snapshot(&active.health);
                let mut router = active.router.lock().await;
                let routes = route_snapshot(&router);
                if let Err(error) = router.stop().await {
                    tracing::warn!(%error, "router stop reported transport cleanup failures");
                }
                *last_health
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal health lock poisoned"))? =
                    health;
                *last_routes
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal route lock poisoned"))? = routes;
                drop(router);
                *inner = None;
                Ok(())
            }),
        )
    }

    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        crate::py_async::future_into_py(py, async move {
            let guard = inner.lock().await;
            let active = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("router not started"))?;
            let router = active.router.lock().await;
            (0..active.health.len())
                .filter(|index| matches!(active.health[*index].1, HealthSource::Bip { .. }))
                .find_map(|index| bip_endpoint(&router, index))
                .map(|address| address.to_string())
                .ok_or_else(|| PyRuntimeError::new_err("router B/IP port is unavailable"))
        })
    }

    fn port_health<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let last = Arc::clone(&self.last_health);
        crate::py_async::future_into_py(py, async move {
            if let Some(active) = inner.lock().await.as_ref() {
                return Ok(health_snapshot(&active.health));
            }
            Ok(last
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal health lock poisoned"))?
                .clone())
        })
    }

    fn routing_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let last = Arc::clone(&self.last_routes);
        crate::py_async::future_into_py(py, async move {
            if let Some(active) = inner.lock().await.as_ref() {
                let router = active.router.lock().await;
                return Ok(route_snapshot(&router));
            }
            Ok(last
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal route lock poisoned"))?
                .clone())
        })
    }
}

fn spawn_background(
    router: &Arc<Mutex<NativeRouter>>,
    health: &[(u16, HealthSource)],
    announce_interval: Option<Duration>,
) -> Vec<JoinHandle<()>> {
    let mut tasks = Vec::new();
    for (index, (_, source)) in health.iter().enumerate() {
        let HealthSource::Sc { state, .. } = source else {
            continue;
        };
        let mut state = state.clone();
        let router = Arc::clone(router);
        tasks.push(tokio::spawn(async move {
            state.borrow_and_update();
            while state.changed().await.is_ok() {
                let current = *state.borrow_and_update();
                // `watch` coalesces rapid Disconnected -> Connected updates.
                // Every post-start notification whose latest state is
                // Connected therefore means the port may have recovered.
                if current == ScConnectionState::Connected {
                    let _ = router.lock().await.reannounce_port(index);
                }
            }
        }));
    }
    if let Some(interval) = announce_interval {
        let router = Arc::clone(router);
        let ports = health.len();
        tasks.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Startup already announced every port.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let router = router.lock().await;
                for index in 0..ports {
                    let _ = router.reannounce_port(index);
                }
            }
        }));
    }
    tasks
}

fn health_snapshot(health: &[(u16, HealthSource)]) -> Vec<PyRouterPortHealth> {
    health
        .iter()
        .enumerate()
        .map(|(index, (network, source))| source.snapshot(index, *network))
        .collect()
}

/// The actual bound IPv4 endpoint of a started B/IP router port, read from the
/// native router's post-start MAC snapshot (the B/IP MAC is the bound
/// address and UDP port, including an ephemeral port selected for port 0).
fn bip_endpoint(router: &NativeRouter, index: usize) -> Option<std::net::SocketAddrV4> {
    let info = router.ports().get(index)?;
    let octets = info.local_mac();
    if octets.len() != 6 {
        return None;
    }
    let ip = std::net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
    let port = u16::from_be_bytes([octets[4], octets[5]]);
    Some(std::net::SocketAddrV4::new(ip, port))
}

fn route_snapshot(router: &NativeRouter) -> Vec<PyRouterRouteEntry> {
    let now = router.now();
    let table = router.table();
    table
        .routes()
        .map(|(network, route)| snapshots::route_entry(network, route, now))
        .collect()
}

async fn build_router_port(
    _config_index: usize,
    configured: PyRouterPortConfig,
) -> PyResult<BuiltPort> {
    let network_number = configured.network_number();
    match configured {
        PyRouterPortConfig::Bip(config) => {
            let interface = config.interface.parse().expect("validated B/IP interface");
            let broadcast = config
                .broadcast_address
                .parse()
                .expect("validated B/IP broadcast address");
            let mut transport = BipTransport::new(interface, config.port, broadcast);
            transport.set_share_port_by_address(config.reuse_port);
            Ok(BuiltPort {
                port: RouterPort {
                    transport: MixedTransport::Bip(Box::new(transport)),
                    network_number,
                },
                health: HealthSource::Bip {
                    identity: format!("{}:{}", config.interface, config.port),
                },
            })
        }
        PyRouterPortConfig::Sc(config) => {
            if config.max_npdu_length != 1497 || config.max_bvlc_length != 5705 {
                return Err(PyValueError::new_err(
                    "upstream 0.13 SC router ports use fixed receive limits of 1497 NPDU / 5705 BVLC",
                ));
            }
            let tls = crate::tls::build_client_tls_config(
                Some(&config.ca_cert),
                Some(&config.client_cert),
                Some(&config.client_key),
            )
            .map_err(|error| PyRuntimeError::new_err(format!("TLS config error: {error}")))?;
            let primary = config.primary_hub.clone();
            let ws = tokio::time::timeout(
                SC_DIAL_TIMEOUT,
                TlsWebSocket::connect(&primary, tls.clone()),
            )
            .await
            .map_err(|_| PyRuntimeError::new_err("SC primary hub dial timed out"))?
            .map_err(to_py_err)?;
            let mut vmac = [0; 6];
            vmac.copy_from_slice(&config.local_vmac);
            let mut uuid = [0; 16];
            uuid.copy_from_slice(&config.device_uuid);
            let connector_url = primary.clone();
            let connector_tls = tls.clone();
            let retries = if config.reconnect_forever {
                u32::MAX
            } else {
                config.reconnect_max_retries
            };
            let transport = ScTransport::new(ws, vmac)
                .with_device_uuid(uuid)
                .with_connect_timeout_ms(SC_DIAL_TIMEOUT.as_millis() as u64)
                .with_heartbeat_interval_ms(config.heartbeat_interval_ms)
                .with_heartbeat_timeout_ms(config.heartbeat_timeout_ms)
                .with_connector(move || {
                    let url = connector_url.clone();
                    let tls = connector_tls.clone();
                    async move { TlsWebSocket::connect(&url, tls).await }
                })
                .with_reconnect(ScReconnectConfig {
                    initial_delay_ms: config.reconnect_initial_delay_ms,
                    max_delay_ms: config.reconnect_max_delay_ms,
                    max_retries: retries,
                });
            let state = transport.connection_state_changes();
            Ok(BuiltPort {
                port: RouterPort {
                    transport: MixedTransport::Sc(Box::new(transport)),
                    network_number,
                },
                health: HealthSource::Sc {
                    identity: primary,
                    state,
                },
            })
        }
    }
}

fn validate_network_number(network: u16) -> PyResult<()> {
    if matches!(network, 0 | u16::MAX) {
        Err(PyValueError::new_err(format!(
            "router network number {network} is reserved"
        )))
    } else {
        Ok(())
    }
}
