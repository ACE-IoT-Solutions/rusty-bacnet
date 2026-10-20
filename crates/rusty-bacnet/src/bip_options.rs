//! Constructor checks for B/IP-only options, and the B/IP mode options
//! (`bbmd=` and `foreign_device=`, #938).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::time::Duration;

use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy, NatTraversal, PrivateRange};
use bacnet_transport::bip::{
    BbmdOptions, BipMode, ForeignDeviceConfig, ForeignRegistrationOutcome,
    ForeignRegistrationSnapshot, DEFAULT_BACNET_PORT,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// `share_port_by_address` (#1538) binds a B/IP interface address, so it
/// means nothing on another transport: refuse it there rather than ignore it.
pub(crate) fn only_on_bip(share_port_by_address: bool, transport: &str) -> PyResult<()> {
    if share_port_by_address && transport != "bip" {
        return Err(PyValueError::new_err(
            "share_port_by_address applies only to transport='bip'",
        ));
    }
    Ok(())
}

/// The shortest renewal interval, in seconds: the transport's pacing floor.
const MIN_RENEWAL_SECS: f64 = 0.1;

/// A foreign-device registration, for the `foreign_device=` keyword (#938).
///
/// `bbmd` is `"ip"` or `"ip:port"` (port 47808 by default). `ttl`, at
/// least 1, is how many seconds the BBMD keeps the entry. `renewal` is the
/// seconds between registrations, at least 0.1 and shorter than `ttl`;
/// `None` renews at half the TTL. Behind NAT, renew often so the mapping the
/// BBMD's replies come back on stays open, while the TTL stays long. A bad
/// value raises ValueError, or OverflowError for an integer out of range.
#[pyclass(
    name = "ForeignDevice",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyForeignDevice {
    config: ForeignDeviceConfig,
}

#[pymethods]
impl PyForeignDevice {
    /// Check and keep the registration's settings.
    #[new]
    #[pyo3(signature = (bbmd, ttl=300, renewal=None))]
    fn new(bbmd: &str, ttl: u16, renewal: Option<f64>) -> PyResult<Self> {
        let address = parse_address(bbmd, "bbmd")?;
        if ttl == 0 {
            return Err(PyValueError::new_err("ttl must be at least 1 second"));
        }
        let renewal_interval = renewal
            .map(|seconds| {
                // The transport renews no faster than every 100 ms.
                if !(seconds >= MIN_RENEWAL_SECS && seconds < f64::from(ttl)) {
                    return Err(PyValueError::new_err(
                        "renewal must be at least 0.1 and less than ttl seconds",
                    ));
                }
                Ok(Duration::from_secs_f64(seconds))
            })
            .transpose()?;
        Ok(Self {
            config: ForeignDeviceConfig {
                bbmd_ip: *address.ip(),
                bbmd_port: address.port(),
                ttl,
                renewal_interval,
            },
        })
    }

    /// The BBMD's address, `"ip:port"`.
    #[getter]
    fn bbmd(&self) -> String {
        SocketAddrV4::new(self.config.bbmd_ip, self.config.bbmd_port).to_string()
    }

    /// The time-to-live, in seconds.
    #[getter]
    fn ttl(&self) -> u16 {
        self.config.ttl
    }

    /// The seconds between registrations, or `None` for half the TTL.
    #[getter]
    fn renewal(&self) -> Option<f64> {
        self.config.renewal_interval.map(|d| d.as_secs_f64())
    }

    fn __repr__(&self) -> String {
        let renewal = self
            .renewal()
            .map_or_else(|| "None".to_string(), |s| s.to_string());
        format!(
            "ForeignDevice(bbmd='{}', ttl={}, renewal={renewal})",
            self.bbmd(),
            self.config.ttl
        )
    }
}

/// A BBMD set-up, for the `bbmd=` keyword (#938).
///
/// `bdt` lists the BBMDs of the other subnets, each `"ip:port"` (its
/// broadcasts forwarded by unicast) or `("ip:port", "mask")`; this BBMD's
/// own row is added when missing. `accept_foreign_devices` lets foreign
/// devices register (default False: every registration is refused).
/// `management_acl` lists the IPv4 addresses allowed to delete Foreign
/// Device Table entries (default: none). `bdt_persist_path` names a file
/// holding a BDT to start with instead, when it loads, and where a BDT
/// activated through the registered Network Port is saved. `alone_on_subnet`
/// says this BBMD is the only device of its B/IP network on its subnet, as
/// behind a NAT router: it then skips the local rebroadcast of forwarded and
/// distributed broadcasts (default False). `nat_global_address`, `"ip"` or
/// `"ip:port"`, runs it behind a NAT router that port-forwards that public
/// address to it (Annex J.7.5): the BDT must then list that address, and
/// every row must be two-hop. `nat_private_ranges` lists up to four
/// `"a.b.c.d/n"` ranges to count as private besides RFC 1918.
#[pyclass(name = "Bbmd", module = "rusty_bacnet", frozen, skip_from_py_object)]
pub(crate) struct PyBbmd {
    options: BbmdOptions,
}

#[pymethods]
impl PyBbmd {
    /// Check and keep the BBMD's settings.
    #[new]
    #[pyo3(signature = (bdt=None, *, accept_foreign_devices=false, management_acl=None, bdt_persist_path=None, alone_on_subnet=false, nat_global_address=None, nat_private_ranges=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        bdt: Option<Vec<Bound<'_, PyAny>>>,
        accept_foreign_devices: bool,
        management_acl: Option<Vec<String>>,
        bdt_persist_path: Option<PathBuf>,
        alone_on_subnet: bool,
        nat_global_address: Option<String>,
        nat_private_ranges: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let bdt = bdt
            .unwrap_or_default()
            .iter()
            .map(bdt_entry)
            .collect::<PyResult<Vec<_>>>()?;
        let management_acl = management_acl
            .unwrap_or_default()
            .iter()
            .map(|ip| parse_ip(ip, "management_acl").map(|ip| ip.octets()))
            .collect::<PyResult<Vec<_>>>()?;
        let nat = nat_options(nat_global_address, nat_private_ranges)?;
        Ok(Self {
            options: BbmdOptions {
                bdt,
                foreign_device_policy: accept_foreign_devices.then(ForeignDevicePolicy::default),
                management_acl,
                bdt_persist_path,
                alone_on_subnet,
                nat,
                ..BbmdOptions::default()
            },
        })
    }

    /// The BDT rows given, as `("ip:port", "mask")`.
    #[getter]
    fn bdt(&self) -> Vec<(String, String)> {
        self.options.bdt.iter().map(bdt_row).collect()
    }

    /// Whether foreign devices may register.
    #[getter]
    fn accept_foreign_devices(&self) -> bool {
        self.options.foreign_device_policy.is_some()
    }

    /// The addresses allowed to delete Foreign Device Table entries.
    #[getter]
    fn management_acl(&self) -> Vec<String> {
        let acl = &self.options.management_acl;
        acl.iter()
            .map(|ip| Ipv4Addr::from(*ip).to_string())
            .collect()
    }

    /// The persisted-BDT file, if any.
    #[getter]
    fn bdt_persist_path(&self) -> Option<PathBuf> {
        self.options.bdt_persist_path.clone()
    }

    /// Whether this BBMD is the only device of its B/IP network on its
    /// subnet.
    #[getter]
    fn alone_on_subnet(&self) -> bool {
        self.options.alone_on_subnet
    }

    /// The NAT router's public `"ip:port"` this BBMD runs behind, if any.
    #[getter]
    fn nat_global_address(&self) -> Option<String> {
        self.options
            .nat
            .as_ref()
            .map(|nat| nat.global_address.to_string())
    }

    /// The ranges counted as private besides RFC 1918, as `"a.b.c.d/n"`.
    #[getter]
    fn nat_private_ranges(&self) -> Vec<String> {
        self.options.nat.as_ref().map_or_else(Vec::new, |nat| {
            nat.private_ranges.iter().map(ToString::to_string).collect()
        })
    }

    fn __repr__(&self) -> String {
        let python = |flag: bool| if flag { "True" } else { "False" };
        let nat = self
            .nat_global_address()
            .map_or_else(|| "None".to_string(), |global| format!("'{global}'"));
        format!(
            "Bbmd(bdt={} rows, accept_foreign_devices={}, alone_on_subnet={}, nat_global_address={nat})",
            self.options.bdt.len(),
            python(self.accept_foreign_devices()),
            python(self.options.alone_on_subnet),
        )
    }
}

/// The B/IP mode the `bbmd=` and `foreign_device=` keywords ask for: one of
/// them at most, and only on B/IP.
pub(crate) fn bip_mode(
    transport: &str,
    bbmd: Option<PyRef<'_, PyBbmd>>,
    foreign_device: Option<PyRef<'_, PyForeignDevice>>,
) -> PyResult<BipMode> {
    if (bbmd.is_some() || foreign_device.is_some()) && transport != "bip" {
        return Err(PyValueError::new_err(
            "bbmd and foreign_device apply only to transport='bip'",
        ));
    }
    match (bbmd, foreign_device) {
        (Some(_), Some(_)) => Err(PyValueError::new_err(
            "bbmd and foreign_device are mutually exclusive: a B/IP node can't be both",
        )),
        (Some(bbmd), None) => Ok(BipMode::Bbmd(bbmd.options.clone())),
        (None, Some(foreign)) => Ok(BipMode::Foreign(foreign.config.clone())),
        (None, None) => Ok(BipMode::Normal),
    }
}

/// `registration` as the dict `foreign_registration()` returns.
pub(crate) fn registration_dict(registration: ForeignRegistrationSnapshot) -> PyResult<Py<PyAny>> {
    let seconds = |d: Option<Duration>| d.map(|d| d.as_secs_f64());
    let outcome = registration.last_outcome.map(|outcome| match outcome {
        ForeignRegistrationOutcome::Pending => "pending",
        ForeignRegistrationOutcome::Accepted => "accepted",
        ForeignRegistrationOutcome::Refused => "refused",
        ForeignRegistrationOutcome::TimedOut => "timed_out",
        ForeignRegistrationOutcome::LocalError => "local_error",
        ForeignRegistrationOutcome::Cancelled => "cancelled",
    });
    crate::py_async::attach(|py| {
        let dict = PyDict::new(py);
        dict.set_item("registered", registration.registered)?;
        dict.set_item("lease_remaining", seconds(registration.lease_remaining))?;
        dict.set_item("next_attempt_in", seconds(registration.next_attempt_in))?;
        dict.set_item("last_bbmd", registration.last_bbmd.map(|a| a.to_string()))?;
        dict.set_item("last_result", registration.last_result.map(|c| c.to_raw()))?;
        dict.set_item("last_outcome", outcome)?;
        dict.set_item("attempts", registration.attempts)?;
        dict.set_item("accepted", registration.accepted)?;
        dict.set_item("refused", registration.refused)?;
        dict.set_item("lapses", registration.lapses)?;
        dict.set_item("renewal_hints", registration.renewal_hints)?;
        Ok(dict.into_any().unbind())
    })
}

/// The NAT setting the `nat_global_address` and `nat_private_ranges`
/// keywords ask for: none without a global address, and ranges only with
/// one.
fn nat_options(
    global: Option<String>,
    ranges: Option<Vec<String>>,
) -> PyResult<Option<NatTraversal>> {
    let Some(global) = global else {
        if ranges.is_some_and(|ranges| !ranges.is_empty()) {
            return Err(PyValueError::new_err(
                "nat_private_ranges needs nat_global_address",
            ));
        }
        return Ok(None);
    };
    let mut nat = NatTraversal::new(parse_address(&global, "nat_global_address")?);
    for range in ranges.unwrap_or_default() {
        nat.private_ranges.push(parse_range(&range)?);
    }
    if nat.private_ranges.len() > NatTraversal::MAX_PRIVATE_RANGES {
        return Err(PyValueError::new_err(format!(
            "nat_private_ranges holds at most {} ranges",
            NatTraversal::MAX_PRIVATE_RANGES
        )));
    }
    Ok(Some(nat))
}

/// `"a.b.c.d/n"`, a prefix of at most 32 bits.
fn parse_range(text: &str) -> PyResult<PrivateRange> {
    let refused = || {
        PyValueError::new_err(format!(
            "nat_private_ranges entries are 'a.b.c.d/n', got '{text}'"
        ))
    };
    let (network, prefix) = text.split_once('/').ok_or_else(refused)?;
    let network = network.parse::<Ipv4Addr>().map_err(|_| refused())?;
    let prefix = prefix.parse::<u8>().map_err(|_| refused())?;
    PrivateRange::new(network, prefix).map_err(|_| refused())
}

/// A BDT row as Python reads it: `("ip:port", "mask")`.
pub(crate) fn bdt_row(row: &BdtEntry) -> (String, String) {
    let address = SocketAddrV4::new(Ipv4Addr::from(row.ip), row.port);
    (
        address.to_string(),
        Ipv4Addr::from(row.broadcast_mask).to_string(),
    )
}

/// A BDT row: `"ip[:port]"`, its broadcasts forwarded by unicast, or
/// `("ip[:port]", "mask")`.
pub(crate) fn bdt_entry(row: &Bound<'_, PyAny>) -> PyResult<BdtEntry> {
    let (address, mask) = match row.extract::<String>() {
        Ok(address) => (address, Ipv4Addr::BROADCAST),
        Err(_) => {
            let (address, mask) = row.extract::<(String, String)>().map_err(|_| {
                PyValueError::new_err("each bdt row is 'ip:port' or ('ip:port', 'mask')")
            })?;
            (address, parse_ip(&mask, "bdt mask")?)
        }
    };
    let address = parse_address(&address, "bdt")?;
    Ok(BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: mask.octets(),
    })
}

/// `"ip"` or `"ip:port"`, a unicast IPv4 address with a nonzero port.
pub(crate) fn parse_address(text: &str, what: &str) -> PyResult<SocketAddrV4> {
    let address = match text.parse::<SocketAddrV4>() {
        Ok(address) => address,
        Err(_) => SocketAddrV4::new(parse_ip(text, what)?, DEFAULT_BACNET_PORT),
    };
    let ip = address.ip();
    if address.port() == 0 || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return Err(PyValueError::new_err(format!(
            "{what} must be a unicast IPv4 address with a nonzero port, got '{text}'"
        )));
    }
    Ok(address)
}

fn parse_ip(text: &str, what: &str) -> PyResult<Ipv4Addr> {
    text.parse()
        .map_err(|_| PyValueError::new_err(format!("{what}: '{text}' is not an IPv4 address")))
}
