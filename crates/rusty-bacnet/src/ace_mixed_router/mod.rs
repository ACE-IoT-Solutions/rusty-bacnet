//! Python-composable BACnet/IP and BACnet/SC router.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use bacnet_network::router::{BACnetRouter as NativeRouter, RouterPort, RouterStopError};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::sc::{
    ScConnectionState, ScEvent, ScHubDialer, ScLinkMonitor, ScReconnectConfig,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::errors::to_py_err;
use crate::router::lifecycle::{CloseError, Lifecycle, StartError};

mod config;
mod publication;
mod session;
mod snapshots;
#[cfg(test)]
mod tests;

use publication::Publication;
use session::MixedSession;

pub use config::{PyRouterBipPort, PyRouterScPort};
use config::{PyRouterPortConfig, validate_ports};
pub use snapshots::{PyRouterPortCounters, PyRouterPortHealth, PyRouterRouteEntry};

type MixedTransport = AnyTransport;
const SC_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
enum HealthSource {
    Bip {
        identity: String,
    },
    Sc {
        identity: String,
        state: watch::Receiver<ScConnectionState>,
        monitor: Option<ScLinkMonitor>,
        projection: Arc<StdMutex<ScHealthProjection>>,
    },
}

#[derive(Clone)]
struct ScHealthProjection {
    since: Instant,
    attempt: u64,
    last_error: Option<String>,
}

impl Default for ScHealthProjection {
    fn default() -> Self {
        Self {
            since: Instant::now(),
            attempt: 0,
            last_error: None,
        }
    }
}

impl HealthSource {
    fn snapshot(&self, config_index: usize, network_number: u16) -> PyRouterPortHealth {
        match self {
            Self::Bip { identity } => {
                snapshots::port_health(config_index, network_number, "bip", identity, None)
            }
            Self::Sc {
                identity,
                state,
                monitor,
                projection,
            } => {
                let metadata = projection
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if let Some(monitor) = monitor {
                    snapshots::port_health_from_status(
                        config_index,
                        network_number,
                        "sc",
                        identity,
                        monitor.connection_status(),
                        metadata.attempt,
                        metadata.last_error.clone(),
                        metadata.since.elapsed().as_secs_f64(),
                    )
                } else {
                    snapshots::port_health(
                        config_index,
                        network_number,
                        "sc",
                        identity,
                        Some(*state.borrow()),
                    )
                }
            }
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
#[pyclass(name = "AceMixedRouter", module = "rusty_bacnet")]
pub struct PyAceMixedRouter {
    lifecycle: Lifecycle<MixedSession>,
    configured_ports: Vec<PyRouterPortConfig>,
    announce_interval: Option<Duration>,
    last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
    last_counters: Arc<StdMutex<Vec<PyRouterPortCounters>>>,
    last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
}

#[pymethods]
impl PyAceMixedRouter {
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
            lifecycle: Lifecycle::new(),
            configured_ports: ports,
            announce_interval,
            last_health: Arc::new(StdMutex::new(Vec::new())),
            last_counters: Arc::new(StdMutex::new(Vec::new())),
            last_routes: Arc::new(StdMutex::new(Vec::new())),
        })
    }

    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = MixedSession::new(
            self.configured_ports.clone(),
            self.announce_interval,
            Arc::clone(&self.last_health),
            Arc::clone(&self.last_counters),
            Arc::clone(&self.last_routes),
        );
        let publication = Publication::new(self.lifecycle.clone());
        let hook = publication.clone();
        crate::py_async::future_into_py_or_put_back(
            py,
            crate::unit_result(
                async move { publication.start(session).await.map_err(start_error) },
            ),
            hook,
        )
    }

    /// Await retained cleanup; failures remain observable and prevent restart.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        crate::py_async::future_into_py(
            py,
            crate::unit_result(async move { lifecycle.close().await.map_err(close_error) }),
        )
    }

    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        crate::py_async::future_into_py(py, async move {
            let (router, indices) = lifecycle
                .inspect(|session| {
                    let active = session.active();
                    let indices: Vec<_> = active
                        .health
                        .iter()
                        .enumerate()
                        .filter_map(|(index, (_, source))| {
                            matches!(source, HealthSource::Bip { .. }).then_some(index)
                        })
                        .collect();
                    (Arc::clone(&active.router), indices)
                })
                .await
                .ok_or_else(|| PyRuntimeError::new_err("router not started"))?;
            let router = router.lock().await;
            indices
                .into_iter()
                .find_map(|index| bip_endpoint(&router, index))
                .map(|address| address.to_string())
                .ok_or_else(|| PyRuntimeError::new_err("router B/IP port is unavailable"))
        })
    }

    fn port_health<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        let last = Arc::clone(&self.last_health);
        crate::py_async::future_into_py(py, async move {
            if let Some(health) = lifecycle
                .inspect(|session| health_snapshot(&session.active().health))
                .await
            {
                return Ok(health);
            }
            Ok(last
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal health lock poisoned"))?
                .clone())
        })
    }

    fn routing_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        let last = Arc::clone(&self.last_routes);
        crate::py_async::future_into_py(py, async move {
            if let Some(router) = lifecycle
                .inspect(|session| Arc::clone(&session.active().router))
                .await
            {
                let router = router.lock().await;
                return Ok(route_snapshot(&router));
            }
            Ok(last
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal route lock poisoned"))?
                .clone())
        })
    }

    fn port_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        let last = Arc::clone(&self.last_counters);
        crate::py_async::future_into_py(py, async move {
            if let Some((router, health)) = lifecycle
                .inspect(|session| {
                    let active = session.active();
                    (Arc::clone(&active.router), active.health.clone())
                })
                .await
            {
                let router = router.lock().await;
                return Ok(counter_snapshot(&router, &health));
            }
            Ok(last
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal counter lock poisoned"))?
                .clone())
        })
    }
}

impl Drop for PyAceMixedRouter {
    fn drop(&mut self) {
        self.lifecycle.dispose();
        let owner = self.lifecycle.clone();
        crate::py_async::spawn_cleanup(async move {
            let _ = owner.close().await;
        });
    }
}

fn close_error(error: CloseError<RouterStopError>) -> PyErr {
    match error {
        CloseError::Stop(error) => PyRuntimeError::new_err(error.to_string()),
        CloseError::WorkerEnded => PyRuntimeError::new_err("router cleanup worker ended"),
    }
}

fn start_error(error: StartError<PyErr, RouterStopError>) -> PyErr {
    match error {
        StartError::Operation(error) => error,
        StartError::Cleanup(error) => close_error(error),
        StartError::AlreadyStarted => {
            PyRuntimeError::new_err("router is already starting or running")
        }
        StartError::Cancelled => PyRuntimeError::new_err("router startup was cancelled"),
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
        // Preserve unread changes since the receiver was created before
        // native startup. An initial Connected may duplicate startup's
        // announcement, but consuming it here could hide an actual recovery.
        let router = Arc::clone(router);
        tasks.push(tokio::spawn(async move {
            while state.changed().await.is_ok() {
                let current = *state.borrow_and_update();
                // `watch` coalesces rapid Disconnected -> Connected updates.
                // Every unread notification whose latest state is
                // Connected therefore means the port may have recovered.
                if current == ScConnectionState::Connected {
                    let _ = router.lock().await.reannounce_port(index);
                }
            }
        }));

        let HealthSource::Sc {
            monitor: Some(monitor),
            projection,
            ..
        } = source
        else {
            continue;
        };
        let monitor = monitor.clone();
        let projection = Arc::clone(projection);
        let mut events = monitor.connection_events();
        tasks.push(tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let mut health =
                            projection.lock().unwrap_or_else(|error| error.into_inner());
                        health.apply(event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // The status snapshot remains authoritative after a
                        // lag. Keep the accumulated diagnostic metadata and
                        // resume at the next event.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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

impl ScHealthProjection {
    fn apply(&mut self, event: ScEvent) {
        self.since = Instant::now();
        match event {
            ScEvent::Connected { .. } | ScEvent::PrimaryRestored => {
                self.attempt = 0;
                self.last_error = None;
            }
            ScEvent::Disconnected { cause, .. }
            | ScEvent::ConnectFailed { cause, .. }
            | ScEvent::PrimaryRestoreFailed { cause, .. } => {
                self.last_error = Some(format!("{cause:?}"));
            }
            ScEvent::ReconnectAttempt { attempt, .. } => {
                self.attempt = u64::from(attempt);
            }
            ScEvent::ReconnectGaveUp { attempts, .. } => {
                self.attempt = u64::from(attempts);
            }
            ScEvent::FailoverEngaged => {}
            _ => {}
        }
    }
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

fn counter_snapshot(
    router: &NativeRouter,
    health: &[(u16, HealthSource)],
) -> Vec<PyRouterPortCounters> {
    router
        .port_counters()
        .into_iter()
        .map(|counter| {
            let index = counter.config_index;
            let mut snapshot = PyRouterPortCounters::from(counter);
            if let Some((_, source)) = health.get(index) {
                snapshot.transport_kind = match source {
                    HealthSource::Bip { .. } => "bip",
                    HealthSource::Sc { .. } => "sc",
                }
                .to_owned();
            }
            snapshot
        })
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
            if interface == std::net::Ipv4Addr::UNSPECIFIED {
                transport.set_share_wildcard_port(config.reuse_port);
            } else {
                transport.set_share_port_by_address(config.reuse_port);
            }
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
                    "SC router ports use fixed receive limits of 1497 NPDU / 5705 BVLC",
                ));
            }
            let tls = crate::tls::build_client_tls_config(
                Some(&config.ca_cert),
                Some(&config.client_cert),
                Some(&config.client_key),
            )
            .map_err(|error| PyRuntimeError::new_err(format!("TLS config error: {error}")))?;
            let primary = config.primary_hub.clone();
            let reconnect = ScReconnectConfig {
                initial_delay_ms: config.reconnect_initial_delay_ms,
                max_delay_ms: config.reconnect_max_delay_ms,
                max_retries: if config.reconnect_forever {
                    u32::MAX
                } else {
                    config.reconnect_max_retries
                },
            };
            let mut hubs = ScHubDialer::new(&primary, tls)
                .connect_timeout_ms(SC_DIAL_TIMEOUT.as_millis() as u64);
            if let Some(failover) = &config.failover_hub {
                hubs = hubs.failover_url(failover);
            }
            let mut vmac = [0; 6];
            vmac.copy_from_slice(&config.local_vmac);
            let mut uuid = [0; 16];
            uuid.copy_from_slice(&config.device_uuid);
            // `ScHubDialer` bounds each primary/failover dial and preserves
            // the selected hub for the transport's initial handshake. Do not
            // wrap the dial in a second total timeout: doing so could expire
            // while the dialer is correctly moving from a failed primary to
            // the failover hub.
            let transport = hubs
                .connect(vmac, Some(reconnect))
                .await
                .map_err(to_py_err)?;
            let transport = transport
                .with_device_uuid(uuid)
                .with_heartbeat_interval_ms(config.heartbeat_interval_ms)
                .with_heartbeat_timeout_ms(config.heartbeat_timeout_ms);
            let state = transport.connection_state_changes();
            let monitor = transport.link_monitor();
            Ok(BuiltPort {
                port: RouterPort {
                    transport: MixedTransport::Sc(Box::new(transport)),
                    network_number,
                },
                health: HealthSource::Sc {
                    identity: primary,
                    state,
                    projection: Arc::new(StdMutex::new(ScHealthProjection::default())),
                    monitor: Some(monitor),
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
