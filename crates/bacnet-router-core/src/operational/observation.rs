//! Exactly-once observation adapters for the typed router boundaries.
//!
//! These methods intentionally observe the result returned by one owner at a
//! time.  They do not inspect a broadcast report, wrap an [`Egress`], or infer
//! a second transmission from a generated rejection descriptor.  The caller
//! therefore has one unambiguous place to record each terminal result.

use super::{DiagnosticEvent, OperationalDiagnostics, OperationalError};
use crate::{
    BroadcastError, BroadcastFanoutOutcome, BroadcastFanoutResult, ControlDelivery, ForwardAttempt,
    ForwardError, ForwardOutcome, PortId, RouterControlAction, RouterControlError,
    RouterControlOutcome,
};

impl<const PORTS: usize, const ROUTES: usize, const CACHE: usize>
    OperationalDiagnostics<PORTS, ROUTES, CACHE>
{
    /// Observe one forwarding invocation, including a result that carries a
    /// source-aware refusal action.
    ///
    /// The refusal is metadata for the caller's response path, not a second
    /// forwarding transmission.  Only `result` is classified, so one call
    /// advances exactly one terminal forwarding counter.
    pub fn observe_forwarding_attempt(
        &mut self,
        port: PortId,
        attempt: ForwardAttempt,
    ) -> Result<(), OperationalError> {
        self.observe_forwarding_result(port, attempt.result)
    }

    /// Observe one ordinary forwarding result.
    pub fn observe_forwarding_result(
        &mut self,
        port: PortId,
        result: Result<ForwardOutcome, ForwardError>,
    ) -> Result<(), OperationalError> {
        let event = match result {
            Ok(ForwardOutcome::Forwarded { .. }) => DiagnosticEvent::Accepted,
            Ok(ForwardOutcome::Oversize { .. }) | Err(ForwardError::ScratchTooSmall { .. }) => {
                DiagnosticEvent::Oversize
            }
            Ok(ForwardOutcome::QueueFull) => DiagnosticEvent::Full,
            Ok(ForwardOutcome::PortDown) => DiagnosticEvent::TransportFailure,
            Ok(ForwardOutcome::Expired) => DiagnosticEvent::Expired,
            Ok(ForwardOutcome::EgressRejected) => DiagnosticEvent::Malformed,
            Err(ForwardError::NoDirectRoute | ForwardError::NoRoute)
            | Err(ForwardError::RouteUnavailable)
            | Err(ForwardError::RouteLoopsToIngress) => DiagnosticEvent::Rejection,
            Err(ForwardError::HopExhausted) => DiagnosticEvent::PolicyDrop,
            Err(
                ForwardError::Truncated
                | ForwardError::UnsupportedVersion(_)
                | ForwardError::MissingDestination
                | ForwardError::InvalidDestinationNetwork(_)
                | ForwardError::AddressTooLong
                | ForwardError::InvalidSource
                | ForwardError::InvalidNetworkMessage
                | ForwardError::PortCapabilityMismatch
                | ForwardError::InvalidEgressAddress,
            ) => DiagnosticEvent::Malformed,
        };
        self.record_event(Some(port), event)
    }

    /// Observe a broadcast operation error raised before target processing.
    ///
    /// The ingress port receives exactly one event.  A successful
    /// [`BroadcastReport`](crate::BroadcastReport) is intentionally not an
    /// input to this API; its aggregate fields must not be added to the
    /// per-target observations.
    pub fn observe_broadcast_error(
        &mut self,
        ingress: PortId,
        error: BroadcastError,
    ) -> Result<(), OperationalError> {
        let event = match error {
            BroadcastError::ResultsTooSmall { .. } => DiagnosticEvent::Full,
            BroadcastError::ScratchTooSmall { .. } => DiagnosticEvent::Oversize,
            BroadcastError::EmptyNpdu
            | BroadcastError::InvalidHopOffset { .. }
            | BroadcastError::InvalidPriority(_)
            | BroadcastError::InvalidOrigin
            | BroadcastError::InvalidNpdu
            | BroadcastError::HopOffsetMismatch { .. } => DiagnosticEvent::Malformed,
        };
        self.record_event(Some(ingress), event)
    }

    /// Observe one broadcast target result.
    ///
    /// Ingress and non-destination entries are selection results rather than
    /// egress attempts and intentionally contribute no counter.  The port is
    /// still validated first, so an unknown port cannot turn a no-op result
    /// into an untracked observation.
    pub fn observe_broadcast_target(
        &mut self,
        result: BroadcastFanoutResult,
    ) -> Result<(), OperationalError> {
        let port = result.port;
        let _ = self.port(port)?;
        let Some(event) = broadcast_event(result.outcome) else {
            return Ok(());
        };
        self.record_event(Some(port), event)
    }

    /// Observe the state-machine result of one network-control message.
    ///
    /// Processing and response admission are separate boundaries.  A
    /// successful outcome records one processing acceptance (or one
    /// rejection for an explicitly unsupported message), while
    /// `capacity_lost` is retained as bounded-capacity evidence.  Call
    /// [`Self::observe_control_delivery`] separately for the resulting wire
    /// response, if any.
    pub fn observe_control_result<'a>(
        &mut self,
        port: PortId,
        result: Result<RouterControlOutcome<'a>, RouterControlError>,
    ) -> Result<(), OperationalError> {
        let event = match result {
            Ok(outcome) => {
                let event = if matches!(outcome.action, RouterControlAction::Unsupported { .. }) {
                    DiagnosticEvent::Rejection
                } else {
                    DiagnosticEvent::Accepted
                };
                self.record_event(Some(port), event)?;
                if outcome.capacity_lost != 0 {
                    self.record_event_count(
                        Some(port),
                        DiagnosticEvent::Full,
                        outcome.capacity_lost as u64,
                    )?;
                }
                return Ok(());
            }
            Err(RouterControlError::ResponseBufferTooSmall { .. })
            | Err(RouterControlError::LearnedTableFull { .. })
            | Err(RouterControlError::SnapshotBufferTooSmall { .. }) => DiagnosticEvent::Full,
            Err(RouterControlError::Codec(_)) | Err(RouterControlError::MissingPeerAddress) => {
                DiagnosticEvent::Malformed
            }
        };
        self.record_event(Some(port), event)
    }

    /// Observe admission of a response action produced by control processing.
    /// `NotNeeded` is not a transmission and contributes no event.
    pub fn observe_control_delivery(
        &mut self,
        port: PortId,
        delivery: ControlDelivery,
    ) -> Result<(), OperationalError> {
        let _ = self.port(port)?;
        match delivery {
            ControlDelivery::NotNeeded => Ok(()),
            ControlDelivery::Accepted => self.record_event(Some(port), DiagnosticEvent::Accepted),
            ControlDelivery::Rejected(outcome) => self.observe_queue_outcome(port, outcome),
        }
    }
}

const fn broadcast_event(outcome: BroadcastFanoutOutcome) -> Option<DiagnosticEvent> {
    match outcome {
        BroadcastFanoutOutcome::IngressExcluded | BroadcastFanoutOutcome::NotDestinationNetwork => {
            None
        }
        BroadcastFanoutOutcome::PolicyDisabled
        | BroadcastFanoutOutcome::CacheUntracked
        | BroadcastFanoutOutcome::HopExhausted => Some(DiagnosticEvent::PolicyDrop),
        BroadcastFanoutOutcome::Duplicate => Some(DiagnosticEvent::Duplicate),
        BroadcastFanoutOutcome::Accepted { .. } => Some(DiagnosticEvent::Accepted),
        BroadcastFanoutOutcome::Full => Some(DiagnosticEvent::Full),
        BroadcastFanoutOutcome::PortDown => Some(DiagnosticEvent::TransportFailure),
        BroadcastFanoutOutcome::Oversize { .. } => Some(DiagnosticEvent::Oversize),
        BroadcastFanoutOutcome::Expired => Some(DiagnosticEvent::Expired),
        BroadcastFanoutOutcome::Invalid => Some(DiagnosticEvent::Malformed),
    }
}
