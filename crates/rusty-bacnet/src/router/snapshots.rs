use bacnet_network::router::RouterPortHealth;
use bacnet_network::router_table::{ReachabilityStatus, RouteSnapshot};
use bacnet_transport::port::TransportHealthState;
use pyo3::prelude::*;

/// Immutable current health for one configured router port.
#[pyclass(name = "RouterPortHealth", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRouterPortHealth {
    #[pyo3(get)]
    pub(super) config_index: usize,
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) transport_kind: String,
    #[pyo3(get)]
    pub(super) identity: String,
    #[pyo3(get)]
    pub(super) state: String,
    #[pyo3(get)]
    pub(super) detail: Option<String>,
    #[pyo3(get)]
    pub(super) active_hub: Option<String>,
    #[pyo3(get)]
    pub(super) last_error: Option<String>,
    #[pyo3(get)]
    pub(super) attempt: u64,
    /// Seconds elapsed since the current state was entered.
    #[pyo3(get)]
    pub(super) since: Option<f64>,
}

impl From<RouterPortHealth> for PyRouterPortHealth {
    fn from(value: RouterPortHealth) -> Self {
        let health = value.health;
        Self {
            config_index: value.config_index,
            network_number: value.network_number,
            transport_kind: value.transport_kind,
            identity: value.identity,
            state: match health.state {
                TransportHealthState::Down => "Down",
                TransportHealthState::Connecting => "Connecting",
                TransportHealthState::Reconnecting => "Reconnecting",
                TransportHealthState::Up => "Up",
                TransportHealthState::Failed => "Failed",
            }
            .to_owned(),
            detail: health.detail,
            active_hub: health.active_hub,
            last_error: health.last_error,
            attempt: health.attempt,
            since: health.since.map(|instant| instant.elapsed().as_secs_f64()),
        }
    }
}

/// Immutable routing-table entry detached from the live native router.
#[pyclass(name = "RouterRouteEntry", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRouterRouteEntry {
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) port_index: usize,
    #[pyo3(get)]
    pub(super) directly_connected: bool,
    #[pyo3(get)]
    pub(super) next_hop_mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) reachability: String,
    #[pyo3(get)]
    pub(super) last_seen_age_s: Option<f64>,
    #[pyo3(get)]
    pub(super) busy_remaining_s: Option<f64>,
    #[pyo3(get)]
    pub(super) flap_count: u8,
    #[pyo3(get)]
    pub(super) last_port_change_age_s: Option<f64>,
}

impl From<RouteSnapshot> for PyRouterRouteEntry {
    fn from(value: RouteSnapshot) -> Self {
        Self {
            network_number: value.network_number,
            port_index: value.port_index,
            directly_connected: value.directly_connected,
            next_hop_mac: value.next_hop_mac,
            reachability: match value.reachability {
                ReachabilityStatus::Reachable => "Reachable",
                ReachabilityStatus::Busy => "Busy",
                ReachabilityStatus::Unreachable => "Unreachable",
            }
            .to_owned(),
            last_seen_age_s: value.last_seen_age.map(|age| age.as_secs_f64()),
            busy_remaining_s: value
                .busy_remaining
                .map(|remaining| remaining.as_secs_f64()),
            flap_count: value.flap_count,
            last_port_change_age_s: value.last_port_change_age.map(|age| age.as_secs_f64()),
        }
    }
}
