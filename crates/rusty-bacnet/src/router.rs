//! Python-composable mixed BACnet/IP and BACnet/SC router.

use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bacnet_network::router::{
    BACnetRouter as NativeRouter, RouterOptions, RouterPort, StartedRouter,
};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::sc::{ScReconnectConfig, ScTransport};
use bacnet_transport::sc_tls::TlsWebSocket;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use tokio::sync::{Mutex, Notify};

use crate::errors::to_py_err;

mod config;
mod snapshots;

use config::{validate_ports, PyRouterPortConfig};
pub use config::{PyRouterBipPort, PyRouterScPort};
pub use snapshots::{PyRouterPortHealth, PyRouterRouteEntry};

type MixedTransport = AnyTransport<crate::mstp_py::PySerial>;
const SC_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

struct StartReservation {
    started: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    token: u64,
    committed: bool,
}

impl Drop for StartReservation {
    fn drop(&mut self) {
        if !self.committed && self.generation.load(Ordering::Acquire) == self.token {
            self.started.store(false, Ordering::Release);
        }
    }
}

struct RestartReservation {
    reserved: Arc<AtomicBool>,
    committed: bool,
}

impl Drop for RestartReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.reserved.store(false, Ordering::Release);
        }
    }
}

struct StopCompletion {
    started: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    token: u64,
    stopping: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Drop for StopCompletion {
    fn drop(&mut self) {
        if self.generation.load(Ordering::Acquire) == self.token {
            self.started.store(false, Ordering::Release);
        }
        self.stopping.store(false, Ordering::Release);
        self.notify.notify_waiters();
    }
}

/// A mixed BACnet/IP and BACnet/SC router.
#[pyclass(name = "BACnetRouter")]
pub struct PyBACnetRouter {
    inner: Arc<Mutex<Option<NativeRouter>>>,
    configured_ports: Vec<PyRouterPortConfig>,
    bip_config_index: Option<usize>,
    started: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    restart_reserved: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    lifecycle: Arc<Mutex<()>>,
    last_health: Arc<StdMutex<Vec<PyRouterPortHealth>>>,
    last_routes: Arc<StdMutex<Vec<PyRouterRouteEntry>>>,
}

#[pymethods]
impl PyBACnetRouter {
    /// Build a mixed-transport router from typed port configurations.
    #[classmethod]
    #[pyo3(signature = (ports))]
    fn from_ports(
        _class: &Bound<'_, pyo3::types::PyType>,
        ports: Vec<PyRouterPortConfig>,
    ) -> PyResult<Self> {
        validate_ports(&ports)?;
        let bip_config_index = ports
            .iter()
            .position(|port| matches!(port, PyRouterPortConfig::Bip(_)));
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            configured_ports: ports,
            bip_config_index,
            started: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            restart_reserved: Arc::new(AtomicBool::new(false)),
            stop_notify: Arc::new(Notify::new()),
            lifecycle: Arc::new(Mutex::new(())),
            last_health: Arc::new(StdMutex::new(Vec::new())),
            last_routes: Arc::new(StdMutex::new(Vec::new())),
        })
    }

    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let waiting_for_stop = if self.stopping.load(Ordering::Acquire) {
            self.restart_reserved
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map(|_| true)
                .map_err(|_| ())
        } else if self.restart_reserved.load(Ordering::Acquire) {
            Err(())
        } else if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Ok(false)
        } else if self.stopping.load(Ordering::Acquire) {
            self.restart_reserved
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map(|_| true)
                .map_err(|_| ())
        } else {
            Err(())
        };
        let waiting_for_stop = waiting_for_stop
            .map_err(|_| PyRuntimeError::new_err("router is already starting or running"))?;
        let inner = Arc::clone(&self.inner);
        let lifecycle = Arc::clone(&self.lifecycle);
        let started = Arc::clone(&self.started);
        let generation = Arc::clone(&self.generation);
        let stopping = Arc::clone(&self.stopping);
        let restart_reserved = Arc::clone(&self.restart_reserved);
        let stop_notify = Arc::clone(&self.stop_notify);
        let configured_ports = self.configured_ports.clone();
        let initial_start_reservation = (!waiting_for_stop).then(|| {
            let token = generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
            StartReservation {
                started: Arc::clone(&started),
                generation: Arc::clone(&generation),
                token,
                committed: false,
            }
        });
        let initial_restart_reservation = waiting_for_stop.then(|| RestartReservation {
            reserved: Arc::clone(&restart_reserved),
            committed: false,
        });
        crate::py_async::future_into_py(py, async move {
            let mut reservation = initial_start_reservation;
            let mut restart_reservation = initial_restart_reservation;
            if waiting_for_stop {
                while stopping.load(Ordering::Acquire) {
                    let notified = stop_notify.notified();
                    if !stopping.load(Ordering::Acquire) {
                        break;
                    }
                    notified.await;
                }
                started
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| PyRuntimeError::new_err("router restart reservation was lost"))?;
                restart_reserved.store(false, Ordering::Release);
                if let Some(reservation) = restart_reservation.as_mut() {
                    reservation.committed = true;
                }
                let token = generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
                reservation = Some(StartReservation {
                    started: Arc::clone(&started),
                    generation: Arc::clone(&generation),
                    token,
                    committed: false,
                });
            }
            let _lifecycle = lifecycle.lock().await;
            let start_token = reservation
                .as_ref()
                .expect("start reservation exists")
                .token;
            if generation.load(Ordering::Acquire) != start_token {
                return Err(PyRuntimeError::new_err(
                    "router start was cancelled by stop",
                ));
            }
            let result: PyResult<()> = async {
                let mut ports = Vec::with_capacity(configured_ports.len());
                for (config_index, configured) in configured_ports.into_iter().enumerate() {
                    ports.push(build_router_port(config_index, configured).await?);
                }
                let StartedRouter { router, .. } = NativeRouter::start(ports, RouterOptions::new())
                    .await
                    .map_err(to_py_err)?;
                *inner.lock().await = Some(router);
                Ok(())
            }
            .await;
            if result.is_ok() {
                reservation
                    .as_mut()
                    .expect("start reservation exists")
                    .committed = true;
            }
            result
        })
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let lifecycle = Arc::clone(&self.lifecycle);
        let started = Arc::clone(&self.started);
        let generation = Arc::clone(&self.generation);
        let stopping = Arc::clone(&self.stopping);
        let stop_notify = Arc::clone(&self.stop_notify);
        let last_health = Arc::clone(&self.last_health);
        let last_routes = Arc::clone(&self.last_routes);
        if stopping.swap(true, Ordering::AcqRel) {
            return crate::py_async::future_into_py(py, async move {
                while stopping.load(Ordering::Acquire) {
                    let notified = stop_notify.notified();
                    if !stopping.load(Ordering::Acquire) {
                        break;
                    }
                    notified.await;
                }
                Ok(())
            });
        }
        let stop_token = generation.load(Ordering::Acquire);
        crate::py_async::future_into_py(py, async move {
            let _completion = StopCompletion {
                started: Arc::clone(&started),
                generation: Arc::clone(&generation),
                token: stop_token,
                stopping,
                notify: stop_notify,
            };
            let _lifecycle = lifecycle.lock().await;
            if generation.load(Ordering::Acquire) != stop_token {
                return Ok(());
            }
            let mut guard = inner.lock().await;
            if let Some(mut router) = guard.take() {
                drop(guard);
                let health_snapshot = router.port_health().into_iter().map(Into::into).collect();
                let route_snapshot = router
                    .routing_table()
                    .await
                    .into_iter()
                    .map(Into::into)
                    .collect();
                router.stop().await;
                *last_health
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))? =
                    health_snapshot;
                *last_routes
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))? =
                    route_snapshot;
            } else if started.load(Ordering::Acquire) {
                generation.store(stop_token.wrapping_add(1), Ordering::Release);
                started.store(false, Ordering::Release);
            }
            Ok(())
        })
    }

    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let bip_config_index = self.bip_config_index;
        crate::py_async::future_into_py(py, async move {
            let guard = inner.lock().await;
            let router = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("router not started"))?;
            router
                .port_health()
                .into_iter()
                .find(|port| Some(port.config_index) == bip_config_index)
                .and_then(|port| bip_identity_to_address(&port.identity))
                .ok_or_else(|| PyRuntimeError::new_err("router B/IP port is unavailable"))
        })
    }

    fn port_health<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let stopping = Arc::clone(&self.stopping);
        let stop_notify = Arc::clone(&self.stop_notify);
        let last_health = Arc::clone(&self.last_health);
        crate::py_async::future_into_py(py, async move {
            if let Some(router) = inner.lock().await.as_ref() {
                return Ok(router.port_health().into_iter().map(Into::into).collect());
            }
            while stopping.load(Ordering::Acquire) {
                let notified = stop_notify.notified();
                if !stopping.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
            Ok(last_health
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?
                .clone())
        })
    }

    fn routing_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let stopping = Arc::clone(&self.stopping);
        let stop_notify = Arc::clone(&self.stop_notify);
        let last_routes = Arc::clone(&self.last_routes);
        crate::py_async::future_into_py(py, async move {
            if let Some(router) = inner.lock().await.as_ref() {
                return Ok(router
                    .routing_table()
                    .await
                    .into_iter()
                    .map(Into::into)
                    .collect());
            }
            while stopping.load(Ordering::Acquire) {
                let notified = stop_notify.notified();
                if !stopping.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
            Ok(last_routes
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?
                .clone())
        })
    }
}

async fn build_router_port(
    config_index: usize,
    configured: PyRouterPortConfig,
) -> PyResult<RouterPort<MixedTransport>> {
    let network_number = configured.network_number();
    let transport = match configured {
        PyRouterPortConfig::Bip(config) => {
            let interface = config.interface.parse().expect("validated B/IP interface");
            let broadcast = config
                .broadcast_address
                .parse()
                .expect("validated B/IP broadcast address");
            let _reuse_port = config.reuse_port;
            let transport = BipTransport::new(interface, config.port, broadcast);
            MixedTransport::Bip(Box::new(transport))
        }
        PyRouterPortConfig::Sc(config) => {
            let tls = crate::tls::build_client_tls_config(
                Some(&config.ca_cert),
                Some(&config.client_cert),
                Some(&config.client_key),
            )
            .map_err(|error| {
                PyRuntimeError::new_err(format!(
                    "router port {config_index} SC TLS configuration for {} failed: {error}",
                    config.primary_hub
                ))
            })?;
            let primary_url = config.primary_hub.clone();
            let initial_dial = sc_dial_with_timeout(
                SC_CONNECT_TIMEOUT,
                TlsWebSocket::connect(&primary_url, tls.clone()),
            )
            .await;
            let ws = match initial_dial {
                Ok(ws) => ws,
                Err(error) if config.failover_hub.is_some() => {
                    TlsWebSocket::from_initial_error(error)
                }
                Err(error) => {
                    return Err(PyRuntimeError::new_err(format!(
                        "router port {config_index} SC hub {primary_url} connection failed: {error}"
                    )))
                }
            };
            let mut vmac = [0; 6];
            vmac.copy_from_slice(&config.local_vmac);
            let mut device_uuid = [0; 16];
            device_uuid.copy_from_slice(&config.device_uuid);
            let reconnect_url = primary_url.clone();
            let reconnect_tls = tls.clone();
            let topology_failover_url = config.failover_hub.clone();
            let mut transport = ScTransport::new(ws, vmac)
                .with_device_uuid(device_uuid)
                .with_hub_urls(&primary_url, topology_failover_url.as_deref())
                .with_heartbeat_interval_ms(config.heartbeat_interval_ms)
                .with_heartbeat_timeout_ms(config.heartbeat_timeout_ms)
                .with_reconnect(ScReconnectConfig {
                    initial_delay_ms: config.reconnect_initial_delay_ms,
                    max_delay_ms: config.reconnect_max_delay_ms,
                    max_retries: config.reconnect_max_retries,
                    retry_forever: config.reconnect_forever,
                })
                .with_connector(move || {
                    let url = reconnect_url.clone();
                    let tls = reconnect_tls.clone();
                    async move { TlsWebSocket::connect(&url, tls).await }
                });
            if let Some(failover_url) = config.failover_hub {
                let failover_tls = tls;
                transport = transport.with_failover_connector(move || {
                    let url = failover_url.clone();
                    let tls = failover_tls.clone();
                    async move { TlsWebSocket::connect(&url, tls).await }
                });
            }
            MixedTransport::Sc(Box::new(transport))
        }
    };
    Ok(RouterPort {
        transport,
        network_number,
    })
}

async fn sc_dial_with_timeout<T, F>(
    timeout: Duration,
    dial: F,
) -> Result<T, bacnet_types::error::Error>
where
    F: Future<Output = Result<T, bacnet_types::error::Error>>,
{
    tokio::time::timeout(timeout, dial)
        .await
        .unwrap_or_else(|_| Err(bacnet_types::error::Error::Timeout(timeout)))
}

fn validate_network_number(network: u16) -> PyResult<()> {
    if network == 0 || network == u16::MAX {
        Err(PyValueError::new_err(format!(
            "router network number {network} is reserved"
        )))
    } else {
        Ok(())
    }
}

fn bip_identity_to_address(identity: &str) -> Option<String> {
    if let Ok(address) = identity.parse::<std::net::SocketAddrV4>() {
        return Some(address.to_string());
    }
    let octets = identity
        .split(':')
        .map(|part| u8::from_str_radix(part, 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (octets.len() == 6).then(|| {
        let ip = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
        let port = u16::from_be_bytes([octets[4], octets[5]]);
        format!("{ip}:{port}")
    })
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[tokio::test]
    async fn stalled_initial_sc_dial_is_bounded() {
        let timeout = Duration::from_millis(25);
        let result = sc_dial_with_timeout::<(), _>(timeout, std::future::pending()).await;
        assert!(
            matches!(result, Err(bacnet_types::error::Error::Timeout(value)) if value == timeout)
        );
    }
}
