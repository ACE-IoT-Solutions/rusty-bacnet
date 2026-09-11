//! Python-composable BACnet router over one B/IP and named virtual ports.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_network::router::{BACnetRouter as NativeRouter, RouterPort, RouterPortCounters};
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::virtual_network::VirtualNetwork;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use tokio::sync::{Mutex, Notify};

use crate::errors::to_py_err;

type MixedTransport = AnyTransport<crate::mstp_py::PySerial>;

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

/// Immutable forwarding counters for one configured router port.
#[pyclass(name = "RouterPortCounters", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRouterPortCounters {
    #[pyo3(get)]
    config_index: usize,
    #[pyo3(get)]
    network_number: u16,
    #[pyo3(get)]
    transport_kind: String,
    #[pyo3(get)]
    identity: String,
    #[pyo3(get)]
    forwarded_unicast: u64,
    #[pyo3(get)]
    forwarded_broadcast: u64,
    #[pyo3(get)]
    decode_drops: u64,
    #[pyo3(get)]
    encode_drops: u64,
    #[pyo3(get)]
    hop_drops: u64,
    #[pyo3(get)]
    no_route_drops: u64,
    #[pyo3(get)]
    busy_drops: u64,
    #[pyo3(get)]
    output_full_drops: u64,
    #[pyo3(get)]
    send_errors: u64,
    #[pyo3(get)]
    shutdown_drops: u64,
}

impl From<RouterPortCounters> for PyRouterPortCounters {
    fn from(value: RouterPortCounters) -> Self {
        Self {
            config_index: value.config_index,
            network_number: value.network_number,
            transport_kind: value.transport_kind,
            identity: value.identity,
            forwarded_unicast: value.forwarded_unicast,
            forwarded_broadcast: value.forwarded_broadcast,
            decode_drops: value.decode_drops,
            encode_drops: value.encode_drops,
            hop_drops: value.hop_drops,
            no_route_drops: value.no_route_drops,
            busy_drops: value.busy_drops,
            output_full_drops: value.output_full_drops,
            send_errors: value.send_errors,
            shutdown_drops: value.shutdown_drops,
        }
    }
}

/// A BACnet router with one BACnet/IPv4 port and one or more virtual ports.
#[pyclass(name = "BACnetRouter")]
pub struct PyBACnetRouter {
    inner: Arc<Mutex<Option<NativeRouter>>>,
    bip_network: u16,
    virtual_ports: Vec<(u16, String, u8)>,
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    reuse_port: bool,
    started: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    restart_reserved: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    lifecycle: Arc<Mutex<()>>,
    last_counters: Arc<StdMutex<Vec<PyRouterPortCounters>>>,
}

#[pymethods]
impl PyBACnetRouter {
    #[new]
    #[pyo3(signature = (
        bip_network,
        virtual_ports,
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        reuse_port=false
    ))]
    fn new(
        bip_network: u16,
        virtual_ports: Vec<(u16, String, u8)>,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        reuse_port: bool,
    ) -> PyResult<Self> {
        if virtual_ports.is_empty() {
            return Err(PyValueError::new_err(
                "virtual_ports must contain at least one (network, name, mac) entry",
            ));
        }
        validate_network_number(bip_network)?;
        let mut networks = HashSet::from([bip_network]);
        let mut names = HashSet::new();
        for (network, name, _) in &virtual_ports {
            validate_network_number(*network)?;
            if !networks.insert(*network) {
                return Err(PyValueError::new_err(format!(
                    "duplicate router network number {network}"
                )));
            }
            if name.trim().is_empty() {
                return Err(PyValueError::new_err(
                    "virtual router network name must not be blank",
                ));
            }
            if !names.insert(name.clone()) {
                return Err(PyValueError::new_err(format!(
                    "duplicate virtual router network name {name:?}"
                )));
            }
        }
        let interface = interface.parse().map_err(|error| {
            PyValueError::new_err(format!("invalid interface IPv4 address: {error}"))
        })?;
        let broadcast_address = broadcast_address.parse().map_err(|error| {
            PyValueError::new_err(format!("invalid broadcast IPv4 address: {error}"))
        })?;
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            bip_network,
            virtual_ports,
            interface,
            port,
            broadcast_address,
            reuse_port,
            started: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            restart_reserved: Arc::new(AtomicBool::new(false)),
            stop_notify: Arc::new(Notify::new()),
            lifecycle: Arc::new(Mutex::new(())),
            last_counters: Arc::new(StdMutex::new(Vec::new())),
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
        let bip_network = self.bip_network;
        let virtual_ports = self.virtual_ports.clone();
        let interface = self.interface;
        let port = self.port;
        let broadcast_address = self.broadcast_address;
        let reuse_port = self.reuse_port;
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
        let task = pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
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
                let mut bip = BipTransport::new(interface, port, broadcast_address);
                bip.set_reuse_port(reuse_port).map_err(to_py_err)?;
                let mut ports = Vec::with_capacity(virtual_ports.len() + 1);
                ports.push(RouterPort {
                    transport: MixedTransport::Bip(bip),
                    network_number: bip_network,
                });
                for (network_number, name, mac) in virtual_ports {
                    ports.push(RouterPort {
                        transport: MixedTransport::Virtual(
                            VirtualNetwork::join(&name, mac).map_err(to_py_err)?,
                        ),
                        network_number,
                    });
                }
                let (router, _local_rx) = NativeRouter::start(ports).await.map_err(to_py_err)?;
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
        });
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            task.await.map_err(|error| {
                PyRuntimeError::new_err(format!("router start task failed: {error}"))
            })?
        })
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let lifecycle = Arc::clone(&self.lifecycle);
        let started = Arc::clone(&self.started);
        let generation = Arc::clone(&self.generation);
        let stopping = Arc::clone(&self.stopping);
        let stop_notify = Arc::clone(&self.stop_notify);
        let last_counters = Arc::clone(&self.last_counters);
        if stopping.swap(true, Ordering::AcqRel) {
            return pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
        let task = pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
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
                router.stop().await;
                let snapshot = router.port_counters().into_iter().map(Into::into).collect();
                *last_counters
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))? = snapshot;
            } else if started.load(Ordering::Acquire) {
                generation.store(stop_token.wrapping_add(1), Ordering::Release);
                started.store(false, Ordering::Release);
            }
            Ok(())
        });
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            task.await.map_err(|error| {
                PyRuntimeError::new_err(format!("router stop task failed: {error}"))
            })?
        })
    }

    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let router = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("router not started"))?;
            router
                .port_counters()
                .into_iter()
                .find(|counter| counter.config_index == 0)
                .and_then(|counter| bip_identity_to_address(&counter.identity))
                .ok_or_else(|| PyRuntimeError::new_err("router B/IP port is unavailable"))
        })
    }

    fn port_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let stopping = Arc::clone(&self.stopping);
        let stop_notify = Arc::clone(&self.stop_notify);
        let last_counters = Arc::clone(&self.last_counters);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if let Some(router) = inner.lock().await.as_ref() {
                return Ok(router.port_counters().into_iter().map(Into::into).collect());
            }
            while stopping.load(Ordering::Acquire) {
                let notified = stop_notify.notified();
                if !stopping.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
            Ok(last_counters
                .lock()
                .map_err(|_| PyRuntimeError::new_err("internal lock poisoned"))?
                .clone())
        })
    }
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
