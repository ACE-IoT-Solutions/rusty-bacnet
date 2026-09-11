//! Python projections for native live BBMD control.

use super::*;
use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bip::{BbmdControl, BbmdControlError, BbmdLifecycle, BbmdSnapshot};
use pyo3::exceptions::{PyRuntimeError, PyValueError};

#[derive(Clone)]
pub(crate) struct PyBbmdTransportConfig {
    pub interface: Ipv4Addr,
    pub port: u16,
    pub broadcast: Ipv4Addr,
    pub reuse_port: bool,
    pub initial_bdt: Vec<BdtEntry>,
    pub persist_path: Option<std::path::PathBuf>,
    pub accept_foreign_devices: bool,
    pub max_fdt_entries: usize,
    pub management_acl: Vec<[u8; 4]>,
    pub policy: Option<super::PythonBvllPolicyBridge>,
}

impl PyBbmdTransportConfig {
    pub(crate) fn transport(&self) -> PyResult<bacnet_transport::bip::BipTransport> {
        let mut transport =
            bacnet_transport::bip::BipTransport::new(self.interface, self.port, self.broadcast);
        transport
            .set_reuse_port(self.reuse_port)
            .map_err(crate::errors::to_py_err)?;
        transport.enable_bbmd(self.initial_bdt.clone());
        if self.accept_foreign_devices {
            transport.enable_foreign_device_registration(
                bacnet_transport::bip::ForeignDevicePolicy::default(),
            );
        }
        transport
            .set_bbmd_accept_foreign_devices(self.accept_foreign_devices)
            .map_err(crate::errors::to_py_err)?;
        transport
            .set_bbmd_max_fdt_entries(self.max_fdt_entries)
            .map_err(crate::errors::to_py_err)?;
        transport.set_bbmd_management_acl(self.management_acl.clone());
        if let Some(path) = &self.persist_path {
            transport.set_bdt_persist_path(path.clone());
        }
        if let Some(policy) = &self.policy {
            transport
                .set_bvll_policy(Arc::new(policy.clone()))
                .map_err(crate::errors::to_py_err)?;
        }
        Ok(transport)
    }
}

fn control_error(error: BbmdControlError) -> PyErr {
    crate::errors::bbmd_control_to_py_err(error)
}

pub(crate) fn parse_bdt_entries(entries: Vec<(String, u16, String)>) -> PyResult<Vec<BdtEntry>> {
    entries
        .into_iter()
        .map(|(ip, port, mask)| {
            if port == 0 {
                return Err(PyValueError::new_err("BDT port must be nonzero"));
            }
            let ip = ip
                .parse::<Ipv4Addr>()
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            let mask = mask
                .parse::<Ipv4Addr>()
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            Ok(BdtEntry {
                ip: ip.octets(),
                port,
                broadcast_mask: mask.octets(),
            })
        })
        .collect()
}

pub(crate) fn parse_management_acl(entries: Vec<String>) -> PyResult<Vec<[u8; 4]>> {
    entries
        .into_iter()
        .map(|ip| {
            ip.parse::<Ipv4Addr>()
                .map(|ip| ip.octets())
                .map_err(|error| PyValueError::new_err(error.to_string()))
        })
        .collect()
}

/// Immutable local BBMD state snapshot.
#[derive(Clone)]
#[pyclass(name = "BbmdCounters", frozen, skip_from_py_object)]
pub struct PyBbmdCounters {
    #[pyo3(get)]
    registrations_accepted: u64,
    #[pyo3(get)]
    registrations_rejected: u64,
    #[pyo3(get)]
    registrations_expired: u64,
    #[pyo3(get)]
    capacity_exhausted: u64,
    #[pyo3(get)]
    management_response_bytes: u64,
    #[pyo3(get)]
    management_responses_throttled: u64,
    #[pyo3(get)]
    fanout_packets_forwarded: u64,
    #[pyo3(get)]
    fanout_packets_throttled: u64,
    #[pyo3(get)]
    fanout_destinations_deduplicated: u64,
    #[pyo3(get)]
    fanout_queue_overflow_drops: u64,
    #[pyo3(get)]
    fanout_send_errors: u64,
}

/// Immutable local BBMD state snapshot.
#[derive(Clone)]
#[pyclass(name = "BbmdSnapshot", frozen, skip_from_py_object)]
pub struct PyBbmdSnapshot {
    #[pyo3(get)]
    revision: u64,
    #[pyo3(get)]
    bdt: Vec<PyBdtEntry>,
    #[pyo3(get)]
    fdt: Vec<PyFdtEntry>,
    #[pyo3(get)]
    accept_foreign_devices: bool,
    #[pyo3(get)]
    max_fdt_entries: usize,
    #[pyo3(get)]
    management_acl: Vec<String>,
    #[pyo3(get)]
    wire_bdt_writes_enabled: bool,
    #[pyo3(get)]
    bdt_persist_path: Option<String>,
    #[pyo3(get)]
    counters: PyBbmdCounters,
}

impl From<BbmdSnapshot> for PyBbmdSnapshot {
    fn from(value: BbmdSnapshot) -> Self {
        let counters = PyBbmdCounters {
            registrations_accepted: value.fdt_counters.registrations_accepted,
            registrations_rejected: value.fdt_counters.registrations_rejected,
            registrations_expired: value.fdt_counters.registrations_expired,
            capacity_exhausted: value.fdt_counters.capacity_exhausted,
            management_response_bytes: value.management_counters.response_bytes_sent,
            management_responses_throttled: value.management_counters.response_throttled,
            fanout_packets_forwarded: value.fanout_counters.packets_forwarded,
            fanout_packets_throttled: value.fanout_counters.packets_throttled,
            fanout_destinations_deduplicated: value.fanout_counters.destinations_deduplicated,
            fanout_queue_overflow_drops: value.fanout_counters.queue_overflow_drops,
            fanout_send_errors: value.fanout_counters.send_errors,
        };
        Self {
            revision: value.revision,
            bdt: value.bdt.into_iter().map(PyBdtEntry::from_rust).collect(),
            fdt: value.fdt.into_iter().map(PyFdtEntry::from_rust).collect(),
            accept_foreign_devices: value.accept_foreign_devices,
            max_fdt_entries: value.max_fdt_entries,
            management_acl: value
                .management_acl
                .into_iter()
                .map(|ip| Ipv4Addr::from(ip).to_string())
                .collect(),
            wire_bdt_writes_enabled: value.wire_bdt_writes_enabled,
            bdt_persist_path: value
                .bdt_persist_path
                .map(|path| path.to_string_lossy().into_owned()),
            counters,
        }
    }
}

/// Capability-limited handle for a local native BBMD.
#[derive(Clone)]
#[pyclass(name = "BbmdControl", frozen, skip_from_py_object)]
pub struct PyBbmdControl {
    inner: BbmdControl,
    policy: Option<super::PythonBvllPolicyBridge>,
}

impl PyBbmdControl {
    pub(crate) fn from_rust(
        inner: BbmdControl,
        policy: Option<super::PythonBvllPolicyBridge>,
    ) -> Self {
        Self { inner, policy }
    }

}

#[pymethods]
impl PyBbmdControl {
    #[getter]
    fn lifecycle(&self) -> &'static str {
        match self.inner.lifecycle() {
            BbmdLifecycle::NotStarted => "not_started",
            BbmdLifecycle::Running => "running",
            BbmdLifecycle::Stopped => "stopped",
        }
    }

    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let control = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            control
                .snapshot()
                .await
                .map(PyBbmdSnapshot::from)
                .map_err(control_error)
        })
    }

    fn policy_counters(&self) -> PyResult<super::PyBvllPolicyCounters> {
        self.policy
            .as_ref()
            .map(super::PythonBvllPolicyBridge::counters)
            .ok_or_else(|| PyRuntimeError::new_err("no Python BVLL policy is configured"))
    }

    fn replace_bdt<'py>(
        &self,
        py: Python<'py>,
        entries: Vec<(String, u16, String)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let entries = parse_bdt_entries(entries)?;
        let control = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            control.replace_bdt(entries).await.map_err(control_error)
        })
    }

    fn set_accept_foreign_devices<'py>(
        &self,
        py: Python<'py>,
        accept: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let control = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            control
                .set_accept_foreign_devices(accept)
                .await
                .map_err(control_error)
        })
    }

    fn set_max_fdt_entries<'py>(
        &self,
        py: Python<'py>,
        maximum: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let control = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            control
                .set_max_fdt_entries(maximum)
                .await
                .map_err(control_error)
        })
    }

    fn set_management_acl<'py>(
        &self,
        py: Python<'py>,
        acl: Vec<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let acl = acl
            .into_iter()
            .map(|ip| {
                ip.parse::<Ipv4Addr>()
                    .map(|ip| ip.octets())
                    .map_err(|error| PyValueError::new_err(error.to_string()))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let control = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            control.set_management_acl(acl).await.map_err(control_error)
        })
    }
}
