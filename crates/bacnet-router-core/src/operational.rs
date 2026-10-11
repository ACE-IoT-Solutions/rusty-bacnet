//! Bounded diagnostics and operational controls for an embedded router.
//!
//! This module is intentionally a small, synchronous observation boundary. It
//! does not own a route table, a duplicate cache, a transport, or a scheduler.
//! A serialized router owner registers its ports, records events at the
//! forwarding/adapter boundaries, supplies route/cache occupancy, and applies
//! the returned control requests to the owning subsystems. All storage is
//! inline and all counters saturate.
//!
//! A [`OperationalSnapshot`] borrows the diagnostic state immutably. The
//! caller must finish using it before mutating this object again. No locking or
//! cross-thread safety is implied; the intended owner is one serialized
//! embedded router task or interrupt-safe handoff chosen by the integration.

use crate::{AdmissionOutcome, PortCapabilities, PortId};

/// The number of independently observable operational counters.
pub const DIAGNOSTIC_COUNTER_COUNT: usize = 9;

/// One class of bounded router diagnostic event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    /// An NPDU or control operation was accepted by its owning boundary.
    Accepted,
    /// A bounded queue or table had no available capacity.
    Full,
    /// The NPDU exceeded the selected data-link or storage profile.
    Oversize,
    /// The request expired before the owning boundary could accept it.
    Expired,
    /// Input failed syntax, framing, or adapter validation.
    Malformed,
    /// A duplicate was suppressed.
    Duplicate,
    /// Policy deliberately prevented forwarding or admission.
    PolicyDrop,
    /// A protocol or operational refusal was generated or observed.
    Rejection,
    /// The underlying transport reported a failure.
    TransportFailure,
}

/// Saturating global or per-port counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticCounters {
    /// Successfully accepted NPDUs or control operations.
    pub accepted: u64,
    /// Capacity failures.
    pub full: u64,
    /// Oversize failures.
    pub oversize: u64,
    /// Expiry failures.
    pub expired: u64,
    /// Malformed or invalid input failures.
    pub malformed: u64,
    /// Duplicate suppression decisions.
    pub duplicate: u64,
    /// Policy drops.
    pub policy_drop: u64,
    /// Explicit protocol or operational rejections.
    pub rejection: u64,
    /// Transport failures.
    pub transport_failure: u64,
}

impl DiagnosticCounters {
    /// A const zero value useful for fixed-capacity arrays.
    pub const ZERO: Self = Self {
        accepted: 0,
        full: 0,
        oversize: 0,
        expired: 0,
        malformed: 0,
        duplicate: 0,
        policy_drop: 0,
        rejection: 0,
        transport_failure: 0,
    };

    /// Record one event with saturating arithmetic.
    pub const fn record(self, event: DiagnosticEvent) -> Self {
        self.record_count(event, 1)
    }

    /// Return a copy with `count` events added using saturating arithmetic.
    pub const fn record_count(mut self, event: DiagnosticEvent, count: u64) -> Self {
        match event {
            DiagnosticEvent::Accepted => self.accepted = self.accepted.saturating_add(count),
            DiagnosticEvent::Full => self.full = self.full.saturating_add(count),
            DiagnosticEvent::Oversize => self.oversize = self.oversize.saturating_add(count),
            DiagnosticEvent::Expired => self.expired = self.expired.saturating_add(count),
            DiagnosticEvent::Malformed => self.malformed = self.malformed.saturating_add(count),
            DiagnosticEvent::Duplicate => self.duplicate = self.duplicate.saturating_add(count),
            DiagnosticEvent::PolicyDrop => {
                self.policy_drop = self.policy_drop.saturating_add(count)
            }
            DiagnosticEvent::Rejection => self.rejection = self.rejection.saturating_add(count),
            DiagnosticEvent::TransportFailure => {
                self.transport_failure = self.transport_failure.saturating_add(count)
            }
        }
        self
    }
}

/// Failure class retained on a registered port until the next failure or
/// explicit diagnostics reset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecentFailure {
    /// Queue capacity was exhausted.
    Full,
    /// NPDU or frame exceeded the active profile.
    Oversize,
    /// The request expired.
    Expired,
    /// Input was malformed or adapter-invalid.
    Malformed,
    /// A duplicate was suppressed.
    Duplicate,
    /// Policy dropped the request.
    PolicyDrop,
    /// A protocol or operational rejection occurred.
    Rejection,
    /// The underlying transport failed or the port was down.
    TransportFailure,
}

impl RecentFailure {
    const fn from_event(event: DiagnosticEvent) -> Option<Self> {
        match event {
            DiagnosticEvent::Accepted => None,
            DiagnosticEvent::Full => Some(RecentFailure::Full),
            DiagnosticEvent::Oversize => Some(RecentFailure::Oversize),
            DiagnosticEvent::Expired => Some(RecentFailure::Expired),
            DiagnosticEvent::Malformed => Some(RecentFailure::Malformed),
            DiagnosticEvent::Duplicate => Some(RecentFailure::Duplicate),
            DiagnosticEvent::PolicyDrop => Some(RecentFailure::PolicyDrop),
            DiagnosticEvent::Rejection => Some(RecentFailure::Rejection),
            DiagnosticEvent::TransportFailure => Some(RecentFailure::TransportFailure),
        }
    }
}

/// Immutable state and queue observations for one registered port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortSnapshot {
    /// Configured capabilities, including the stable port identifier.
    pub capabilities: PortCapabilities,
    /// Whether the operational control plane currently permits this port.
    pub enabled: bool,
    /// Most recent queue result observed for this port.
    pub last_queue_outcome: Option<AdmissionOutcome>,
    /// Most recent failure observed for this port, if any.
    pub recent_failure: Option<RecentFailure>,
}

/// Current used/capacity pair for a route table or duplicate cache.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OccupancySnapshot {
    /// Number of currently occupied entries supplied by the owning subsystem.
    pub used: usize,
    /// Fixed capacity represented by this operational instance.
    pub capacity: usize,
}

/// A read-only, allocation-free view of operational state.
///
/// The slices borrow the [`OperationalDiagnostics`] object. They are valid
/// only until the next mutable operation on that object. `None` slots are
/// unregistered port capacity and contain no port state or counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationalSnapshot<'a> {
    /// Registered port state, in deterministic registration-slot order.
    pub ports: &'a [Option<PortSnapshot>],
    /// Per-slot counters corresponding to [`Self::ports`].
    pub per_port: &'a [DiagnosticCounters],
    /// Detached global counters at snapshot time.
    pub global: DiagnosticCounters,
    /// Route-table occupancy supplied by the route owner.
    pub route_occupancy: OccupancySnapshot,
    /// Duplicate-cache occupancy supplied by the policy owner.
    pub cache_occupancy: OccupancySnapshot,
}

impl<'a> OperationalSnapshot<'a> {
    /// Return one registered port and its counters by stable identifier.
    pub fn port(self, id: PortId) -> Option<(PortSnapshot, DiagnosticCounters)> {
        self.ports
            .iter()
            .zip(self.per_port.iter())
            .find_map(|(slot, counters)| {
                slot.map(|port| (port, *counters))
                    .filter(|(port, _)| port.capabilities.id == id)
            })
    }

    /// Count registered ports in this snapshot.
    pub fn registered_ports(self) -> usize {
        self.ports.iter().flatten().count()
    }
}

/// Operational controls consumed synchronously by the serialized router owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationalControl {
    /// Enable or disable one registered port. The adapter must enforce the
    /// returned state at its actual ingress/egress boundary.
    SetPortEnabled { port: PortId, enabled: bool },
    /// Ask the route owner to remove all learned routes.
    ClearLearnedRoutes,
    /// Ask the route/discovery owner to refresh learned routes.
    RequestLearnedRouteRefresh,
    /// Ask the caller to emit its configured discovery announcement.
    AnnounceDiscovery,
    /// Reset counters and per-port recent failure observations while
    /// preserving registrations, enable state, and occupancy inputs.
    ResetDiagnostics,
}

/// Result of applying one operational control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationalControlResult {
    /// Port state after the operation. `changed` is false for an idempotent
    /// request.
    PortState {
        port: PortId,
        enabled: bool,
        changed: bool,
    },
    /// The caller must clear learned route state in its route owner.
    LearnedRoutesClearRequested,
    /// The caller must initiate a learned-route refresh/discovery exchange.
    LearnedRoutesRefreshRequested,
    /// The caller must emit its discovery announcement through the transport.
    DiscoveryAnnouncementRequested,
    /// Counters and recent failures were reset.
    DiagnosticsReset,
}

/// Why an operational request is invalid even though its type is well formed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidState {
    /// A port identifier was registered twice.
    PortAlreadyRegistered(PortId),
    /// A port advertised no possible NPDU capacity.
    ZeroPortNpduCapacity(PortId),
    /// Route occupancy exceeded this instance's fixed route capacity.
    RouteOccupancyExceedsCapacity { used: usize, capacity: usize },
    /// Cache occupancy exceeded this instance's fixed cache capacity.
    CacheOccupancyExceedsCapacity { used: usize, capacity: usize },
}

/// A bounded operational API failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationalError {
    /// No registration slot remained.
    PortTableFull { capacity: usize },
    /// A control or observation named no registered port.
    UnknownPort(PortId),
    /// The request conflicts with the current bounded state.
    InvalidState(InvalidState),
}

/// Bounded, synchronous diagnostics and controls.
///
/// `PORTS` is the number of registered-port slots. `ROUTES` and `CACHE` are
/// capacities used to validate the occupancy inputs; route and cache storage
/// remain owned by their respective subsystems. The object has no allocator,
/// lock, clock, or transport dependency.
#[derive(Clone, Debug)]
pub struct OperationalDiagnostics<const PORTS: usize, const ROUTES: usize, const CACHE: usize> {
    ports: [Option<PortSnapshot>; PORTS],
    per_port: [DiagnosticCounters; PORTS],
    global: DiagnosticCounters,
    route_occupancy: OccupancySnapshot,
    cache_occupancy: OccupancySnapshot,
}

impl<const PORTS: usize, const ROUTES: usize, const CACHE: usize>
    OperationalDiagnostics<PORTS, ROUTES, CACHE>
{
    /// Construct an empty diagnostics/control plane.
    pub const fn new() -> Self {
        Self {
            ports: [None; PORTS],
            per_port: [DiagnosticCounters::ZERO; PORTS],
            global: DiagnosticCounters::ZERO,
            route_occupancy: OccupancySnapshot {
                used: 0,
                capacity: ROUTES,
            },
            cache_occupancy: OccupancySnapshot {
                used: 0,
                capacity: CACHE,
            },
        }
    }

    /// Register one port in the first available deterministic slot.
    pub fn register_port(
        &mut self,
        capabilities: PortCapabilities,
    ) -> Result<(), OperationalError> {
        if capabilities.max_npdu_len == 0 {
            return Err(OperationalError::InvalidState(
                InvalidState::ZeroPortNpduCapacity(capabilities.id),
            ));
        }
        if self.port_index(capabilities.id).is_some() {
            return Err(OperationalError::InvalidState(
                InvalidState::PortAlreadyRegistered(capabilities.id),
            ));
        }
        let Some(index) = self.ports.iter().position(Option::is_none) else {
            return Err(OperationalError::PortTableFull { capacity: PORTS });
        };
        self.ports[index] = Some(PortSnapshot {
            capabilities,
            enabled: true,
            last_queue_outcome: None,
            recent_failure: None,
        });
        Ok(())
    }

    /// Remove one port registration and clear its per-slot observations.
    pub fn unregister_port(&mut self, port: PortId) -> Result<(), OperationalError> {
        let index = self
            .port_index(port)
            .ok_or(OperationalError::UnknownPort(port))?;
        self.ports[index] = None;
        self.per_port[index] = DiagnosticCounters::ZERO;
        Ok(())
    }

    /// Return a read-only snapshot with no allocation or copying of arrays.
    pub fn snapshot(&self) -> OperationalSnapshot<'_> {
        OperationalSnapshot {
            ports: &self.ports,
            per_port: &self.per_port,
            global: self.global,
            route_occupancy: self.route_occupancy,
            cache_occupancy: self.cache_occupancy,
        }
    }

    /// Return global counters as a detached copy.
    pub const fn global_counters(&self) -> DiagnosticCounters {
        self.global
    }

    /// Return one registered port's state and counters as detached copies.
    pub fn port(&self, id: PortId) -> Result<(PortSnapshot, DiagnosticCounters), OperationalError> {
        let index = self
            .port_index(id)
            .ok_or(OperationalError::UnknownPort(id))?;
        Ok((
            self.ports[index].expect("registered index has a port"),
            self.per_port[index],
        ))
    }

    /// Return whether a port is registered and enabled.
    pub fn is_port_enabled(&self, port: PortId) -> Result<bool, OperationalError> {
        Ok(self.port(port)?.0.enabled)
    }

    /// Return the number of registered ports.
    pub fn registered_ports(&self) -> usize {
        self.ports.iter().flatten().count()
    }

    /// Supply route occupancy from the route-table owner.
    pub fn set_route_occupancy(&mut self, used: usize) -> Result<(), OperationalError> {
        if used > ROUTES {
            return Err(OperationalError::InvalidState(
                InvalidState::RouteOccupancyExceedsCapacity {
                    used,
                    capacity: ROUTES,
                },
            ));
        }
        self.route_occupancy.used = used;
        Ok(())
    }

    /// Supply duplicate-cache occupancy from the broadcast-policy owner.
    pub fn set_cache_occupancy(&mut self, used: usize) -> Result<(), OperationalError> {
        if used > CACHE {
            return Err(OperationalError::InvalidState(
                InvalidState::CacheOccupancyExceedsCapacity {
                    used,
                    capacity: CACHE,
                },
            ));
        }
        self.cache_occupancy.used = used;
        Ok(())
    }

    /// Record one global event or one event for a registered port.
    ///
    /// `None` records only the global counter. `Some(port)` records both the
    /// global and the matching per-port counter. Disabled ports may still be
    /// observed: this permits the adapter to report the rejection that caused
    /// a disabled-port decision without silently losing diagnostics.
    pub fn record_event(
        &mut self,
        port: Option<PortId>,
        event: DiagnosticEvent,
    ) -> Result<(), OperationalError> {
        self.record_event_count(port, event, 1)
    }

    /// Record `count` copies of an event with saturating counters.
    pub fn record_event_count(
        &mut self,
        port: Option<PortId>,
        event: DiagnosticEvent,
        count: u64,
    ) -> Result<(), OperationalError> {
        let index = port
            .map(|id| self.port_index(id).ok_or(OperationalError::UnknownPort(id)))
            .transpose()?;
        if count == 0 {
            return Ok(());
        }
        self.global = self.global.record_count(event, count);
        if let Some(index) = index {
            self.per_port[index] = self.per_port[index].record_count(event, count);
            if let Some(snapshot) = self.ports[index].as_mut() {
                if let Some(failure) = RecentFailure::from_event(event) {
                    snapshot.recent_failure = Some(failure);
                }
            }
        }
        Ok(())
    }

    /// Observe a queue result and update its matching queue/failure counters.
    pub fn observe_queue_outcome(
        &mut self,
        port: PortId,
        outcome: AdmissionOutcome,
    ) -> Result<(), OperationalError> {
        let index = self
            .port_index(port)
            .ok_or(OperationalError::UnknownPort(port))?;
        let event = match outcome {
            AdmissionOutcome::AcceptedOwned => DiagnosticEvent::Accepted,
            AdmissionOutcome::Full => DiagnosticEvent::Full,
            AdmissionOutcome::Oversize => DiagnosticEvent::Oversize,
            AdmissionOutcome::Expired => DiagnosticEvent::Expired,
            AdmissionOutcome::PortDown => DiagnosticEvent::TransportFailure,
            AdmissionOutcome::Invalid => DiagnosticEvent::Malformed,
        };
        self.global = self.global.record(event);
        self.per_port[index] = self.per_port[index].record(event);
        let snapshot = self.ports[index]
            .as_mut()
            .expect("registered index has a port");
        snapshot.last_queue_outcome = Some(outcome);
        if let Some(failure) = RecentFailure::from_event(event) {
            snapshot.recent_failure = Some(failure);
        }
        Ok(())
    }

    /// Apply one operational control synchronously.
    ///
    /// Route clearing, route refresh, and discovery announcement are requests
    /// only. The caller remains responsible for invoking the route owner or
    /// transport and recording any resulting event or queue outcome.
    pub fn apply(
        &mut self,
        control: OperationalControl,
    ) -> Result<OperationalControlResult, OperationalError> {
        match control {
            OperationalControl::SetPortEnabled { port, enabled } => {
                let index = self
                    .port_index(port)
                    .ok_or(OperationalError::UnknownPort(port))?;
                let snapshot = self.ports[index]
                    .as_mut()
                    .expect("registered index has a port");
                let changed = snapshot.enabled != enabled;
                snapshot.enabled = enabled;
                Ok(OperationalControlResult::PortState {
                    port,
                    enabled,
                    changed,
                })
            }
            OperationalControl::ClearLearnedRoutes => {
                Ok(OperationalControlResult::LearnedRoutesClearRequested)
            }
            OperationalControl::RequestLearnedRouteRefresh => {
                Ok(OperationalControlResult::LearnedRoutesRefreshRequested)
            }
            OperationalControl::AnnounceDiscovery => {
                Ok(OperationalControlResult::DiscoveryAnnouncementRequested)
            }
            OperationalControl::ResetDiagnostics => {
                self.reset_diagnostics();
                Ok(OperationalControlResult::DiagnosticsReset)
            }
        }
    }

    /// Reset global/per-port counters and recent observations while retaining
    /// port registration, enabled state, and route/cache occupancy.
    pub fn reset_diagnostics(&mut self) {
        self.global = DiagnosticCounters::ZERO;
        for counter in &mut self.per_port {
            *counter = DiagnosticCounters::ZERO;
        }
        for snapshot in &mut self.ports {
            if let Some(snapshot) = snapshot.as_mut() {
                snapshot.last_queue_outcome = None;
                snapshot.recent_failure = None;
            }
        }
    }

    fn port_index(&self, id: PortId) -> Option<usize> {
        self.ports
            .iter()
            .position(|slot| slot.is_some_and(|snapshot| snapshot.capabilities.id == id))
    }
}

impl<const PORTS: usize, const ROUTES: usize, const CACHE: usize> Default
    for OperationalDiagnostics<PORTS, ROUTES, CACHE>
{
    fn default() -> Self {
        Self::new()
    }
}
