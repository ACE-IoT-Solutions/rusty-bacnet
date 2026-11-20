use bacnet_network::router::RouterPortCounters;
use bacnet_network::router_table::{Duration, Instant, Reachability, Route};
use bacnet_transport::sc::{ScActiveHub, ScConnectionState, ScLinkStatus};
use pyo3::prelude::*;

/// Immutable cumulative forwarding counters for one configured router port.
#[pyclass(
    name = "RouterPortCounters",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub struct PyRouterPortCounters {
    #[pyo3(get)]
    pub(super) config_index: usize,
    #[pyo3(get)]
    pub(super) network_number: u16,
    #[pyo3(get)]
    pub(super) transport_kind: String,
    #[pyo3(get)]
    pub(super) identity: String,
    #[pyo3(get)]
    pub(super) forwarded_unicast: u64,
    #[pyo3(get)]
    pub(super) forwarded_broadcast: u64,
    #[pyo3(get)]
    pub(super) send_errors: u64,
    #[pyo3(get)]
    pub(super) output_full_drops: u64,
    #[pyo3(get)]
    pub(super) no_route_drops: u64,
    #[pyo3(get)]
    pub(super) shutdown_drops: u64,
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
            send_errors: value.send_errors,
            output_full_drops: value.output_full_drops,
            no_route_drops: value.no_route_drops,
            shutdown_drops: value.shutdown_drops,
        }
    }
}

/// Immutable current health for one configured router port.
#[pyclass(
    name = "RouterPortHealth",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
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

impl PyRouterPortHealth {
    /// Build a health snapshot from the configured port and its live SC state.
    ///
    /// B/IP is stateless in the upstream transport and is therefore always
    /// reported as `Up`. SC state is projected without inventing retry/error
    /// metadata that upstream 0.13 no longer exposes through the router.
    pub(super) fn from_port(
        config_index: usize,
        network_number: u16,
        transport_kind: &str,
        identity: String,
        sc_state: Option<ScConnectionState>,
    ) -> Self {
        let state = match sc_state {
            None => "Up",
            Some(ScConnectionState::Disconnected | ScConnectionState::Disconnecting) => "Down",
            Some(ScConnectionState::Connecting) => "Connecting",
            Some(ScConnectionState::Connected) => "Up",
        };
        Self {
            config_index,
            network_number,
            transport_kind: transport_kind.to_owned(),
            identity,
            state: state.to_owned(),
            detail: None,
            active_hub: None,
            last_error: None,
            attempt: 0,
            since: None,
        }
    }

    pub(super) fn from_sc_status(
        config_index: usize,
        network_number: u16,
        identity: String,
        status: ScLinkStatus,
        attempt: u64,
        last_error: Option<String>,
        since: f64,
    ) -> Self {
        let state = match status.state {
            ScConnectionState::Disconnected | ScConnectionState::Disconnecting => "Down",
            ScConnectionState::Connecting if attempt > 0 => "Reconnecting",
            ScConnectionState::Connecting => "Connecting",
            ScConnectionState::Connected => "Up",
        };
        Self {
            config_index,
            network_number,
            transport_kind: "sc".to_owned(),
            identity,
            state: state.to_owned(),
            detail: None,
            active_hub: status.active_hub.map(sc_hub_name),
            last_error,
            attempt,
            since: Some(since),
        }
    }
}

fn sc_hub_name(hub: ScActiveHub) -> String {
    match hub {
        ScActiveHub::Primary => "primary".to_owned(),
        ScActiveHub::Failover => "failover".to_owned(),
    }
}

/// Build a health snapshot for the parent router's configured health source.
pub(super) fn port_health(
    config_index: usize,
    network_number: u16,
    transport_kind: &str,
    identity: &str,
    sc_state: Option<ScConnectionState>,
) -> PyRouterPortHealth {
    PyRouterPortHealth::from_port(
        config_index,
        network_number,
        transport_kind,
        identity.to_owned(),
        sc_state,
    )
}

pub(super) fn port_health_from_status(
    config_index: usize,
    network_number: u16,
    _transport_kind: &str,
    identity: &str,
    status: ScLinkStatus,
    attempt: u64,
    last_error: Option<String>,
    since: f64,
) -> PyRouterPortHealth {
    PyRouterPortHealth::from_sc_status(
        config_index,
        network_number,
        identity.to_owned(),
        status,
        attempt,
        last_error,
        since,
    )
}

/// Immutable routing-table entry detached from the live native router.
#[pyclass(
    name = "RouterRouteEntry",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
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

impl PyRouterRouteEntry {
    /// Copy one upstream route row into the Python-facing snapshot contract.
    pub(super) fn from_route(network_number: u16, route: &Route, now: Instant) -> Self {
        let reachability = effective_reachability(route, now);
        let busy_remaining_s = (reachability == Reachability::Busy)
            .then(|| route.busy_until())
            .flatten()
            .and_then(|deadline| deadline.checked_duration_since(now))
            .map(duration_seconds);
        Self {
            network_number,
            port_index: usize::from(route.port().0),
            directly_connected: route.is_direct(),
            next_hop_mac: route.next_hop().as_slice().to_vec(),
            reachability: reachability_name(reachability).to_owned(),
            last_seen_age_s: route
                .last_seen()
                .and_then(|stamp| now.checked_duration_since(stamp))
                .map(duration_seconds),
            busy_remaining_s,
            flap_count: route.flap_count(),
            last_port_change_age_s: route
                .last_port_change()
                .and_then(|stamp| now.checked_duration_since(stamp))
                .map(duration_seconds),
        }
    }
}

/// Compatibility spelling used by the Python router owner.
pub(super) fn route_entry(network_number: u16, route: &Route, now: Instant) -> PyRouterRouteEntry {
    PyRouterRouteEntry::from_route(network_number, route, now)
}

fn effective_reachability(route: &Route, now: Instant) -> Reachability {
    match (route.reachability(), route.busy_until()) {
        (Reachability::Busy, Some(deadline)) if now >= deadline => Reachability::Reachable,
        (state, _) => state,
    }
}

fn reachability_name(value: Reachability) -> &'static str {
    match value {
        Reachability::Reachable => "Reachable",
        Reachability::Busy => "Busy",
        Reachability::Unreachable => "Unreachable",
    }
}

fn duration_seconds(duration: Duration) -> f64 {
    duration.as_nanos() as f64 / 1_000_000_000.0
}
