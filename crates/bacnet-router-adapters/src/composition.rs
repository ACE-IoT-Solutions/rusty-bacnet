//! Bounded composition seams for a B/IP ↔ router ↔ MS/TP path.
//!
//! This module composes the strict B/IP codec, the synchronous router, and the
//! concrete link adapters without adding a queue of its own.  Every operation
//! is caller-buffered and completes synchronously: a successful egress
//! admission transfers ownership to the selected data-link queue, while every
//! other result leaves the caller's NPDU available for its policy decision.

use bacnet_mstp_core::{
    Action, CoreError, Instant, MasterCore, QueueStorage, ReplyDecision, RequestId,
};
use bacnet_router_core::{
    BipDecodeError, DirectForwardRequest, Egress, ForwardError, ForwardOutcome, Ipv4SocketAddress,
    LinkAddress, NetworkNumber, PortCapabilities, RouterCore,
};

use crate::bip::{decode_ingress, BipEgress, BipIngressContext};

/// Failure while composing one link ingress with the router.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionError {
    /// The complete UDP datagram failed strict BVLL validation.
    BipDecode(BipDecodeError),
    /// The router could not parse or route the NPDU.
    Forward(ForwardError),
    /// A non-delivery MS/TP action was supplied to the reverse seam.
    InvalidAction,
    /// A B/IP egress was requested for a port that is not B/IP.
    InvalidBipPort,
    /// The bounded application reply binding could not be reserved.
    Binding(BindingAdmission),
}

/// Fixed-capacity application binding admission result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingAdmission {
    /// The request was inserted into a free binding slot.
    Bound,
    /// No binding slot is available.
    Full,
    /// The caller's clock regressed relative to a prior operation.
    Expired,
    /// The request identifier is already bound or otherwise invalid.
    Invalid,
}

/// The result of a reply or timeout operation against a bound MS/TP request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingCompletionError {
    /// No matching binding exists.
    Invalid,
    /// The caller's clock regressed or the bounded reply window expired.
    Expired,
    /// The MS/TP core rejected the requested decision.
    Core(CoreError),
}

/// One fixed-capacity binding between an MS/TP request generation and the
/// application that received the routed NPDU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BindingSlot {
    id: RequestId,
    expires_at: u64,
}

/// Bounded request/reply ownership for `Action::DeliverNpdu` request IDs.
///
/// The MS/TP core remains the authority for the actual Clause 9 reply state.
/// This object only retains the generation long enough for an application
/// response or timeout to call [`MasterCore::complete_request`].  It stores no
/// NPDU and has no retry/backlog behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestBindings<const CAPACITY: usize> {
    slots: [Option<BindingSlot>; CAPACITY],
    reply_window: u64,
    last_now: u64,
}

impl<const CAPACITY: usize> RequestBindings<CAPACITY> {
    /// Construct empty bindings with a caller-defined application reply
    /// window, in the same monotonic units as router timestamps.
    pub const fn new(reply_window: u64, now: u64) -> Self {
        Self {
            slots: [None; CAPACITY],
            reply_window,
            last_now: now,
        }
    }

    /// Number of currently retained request generations.
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Whether no request generations are retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retain one request ID for a bounded application response window.
    pub fn bind(&mut self, id: RequestId, now: u64) -> BindingAdmission {
        if now < self.last_now {
            return BindingAdmission::Expired;
        }
        self.last_now = now;
        self.expire(now);
        if self.slots.iter().flatten().any(|slot| slot.id == id) {
            return BindingAdmission::Invalid;
        }
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) else {
            return BindingAdmission::Full;
        };
        *slot = Some(BindingSlot {
            id,
            expires_at: now.saturating_add(self.reply_window),
        });
        BindingAdmission::Bound
    }

    /// Remove one binding without calling the MS/TP core.  This is used when
    /// router admission fails after a tentative request binding.
    pub fn unbind(&mut self, id: RequestId) -> bool {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_some_and(|slot| slot.id == id))
        {
            *slot = None;
            true
        } else {
            false
        }
    }

    /// Complete a retained request with the application's reply decision.
    ///
    /// The NPDU is passed directly to `MasterCore`; this layer never queues or
    /// copies it.  A successful decision consumes the binding.  A core error
    /// is returned to the caller so oversize replies and stale generations are
    /// not silently reported as delivered.
    pub fn complete<'rx, Q: QueueStorage>(
        &mut self,
        core: &mut MasterCore<'rx, Q>,
        id: RequestId,
        decision: ReplyDecision<'_>,
        now: Instant,
    ) -> Result<(), BindingCompletionError> {
        let micros = now.as_micros();
        if micros < self.last_now {
            return Err(BindingCompletionError::Expired);
        }
        self.last_now = micros;
        if !self.slots.iter().flatten().any(|slot| slot.id == id) {
            return Err(BindingCompletionError::Invalid);
        }
        self.complete_decision(core, id, decision, now, false)
    }

    /// Apply a decision while permitting the adapter window to have elapsed.
    /// This is used only by [`Self::timeout`], so the MS/TP core can perform
    /// its own deadline transition rather than leaving a pending generation.
    fn complete_decision<'rx, Q: QueueStorage>(
        &mut self,
        core: &mut MasterCore<'rx, Q>,
        id: RequestId,
        decision: ReplyDecision<'_>,
        now: Instant,
        permit_expired: bool,
    ) -> Result<(), BindingCompletionError> {
        let micros = now.as_micros();
        if micros < self.last_now {
            return Err(BindingCompletionError::Expired);
        }
        self.last_now = micros;
        let Some(slot) = self
            .slots
            .iter()
            .flatten()
            .find(|slot| slot.id == id)
            .copied()
        else {
            return Err(BindingCompletionError::Invalid);
        };
        if !permit_expired && micros >= slot.expires_at {
            self.unbind(id);
            return Err(BindingCompletionError::Expired);
        }
        match core.complete_request(id, decision, now) {
            Ok(()) => {
                self.unbind(id);
                Ok(())
            }
            Err(
                error @ (CoreError::NoPendingRequest
                | CoreError::StaleRequest
                | CoreError::RequestExpired),
            ) => {
                self.unbind(id);
                Err(BindingCompletionError::Core(error))
            }
            Err(error) => Err(BindingCompletionError::Core(error)),
        }
    }

    /// Abandon a retained request after the application-side timeout.
    ///
    /// The core is still called, so it can apply its own reply-window and
    /// state-machine rules.  No response NPDU is generated by this decision.
    pub fn timeout<'rx, Q: QueueStorage>(
        &mut self,
        core: &mut MasterCore<'rx, Q>,
        id: RequestId,
        now: Instant,
    ) -> Result<(), BindingCompletionError> {
        self.complete_decision(core, id, ReplyDecision::Abandoned, now, true)
    }

    fn expire(&mut self, now: u64) {
        for slot in &mut self.slots {
            if slot.is_some_and(|slot| now >= slot.expires_at) {
                *slot = None;
            }
        }
    }
}

/// Decode a complete B/IP datagram and route its logical NPDU to MS/TP.
///
/// The strict decoder runs before any router diagnostic or egress mutation.
/// Forwarded-NPDU source attribution is taken from the embedded origin through
/// [`BipIngressContext`], never from the immediate BBMD UDP peer.
#[allow(clippy::too_many_arguments)]
pub fn forward_bip_datagram_to_mstp<'a, const DIRECT: usize, const LEARNED: usize>(
    router: &mut RouterCore<DIRECT, LEARNED>,
    datagram: &'a [u8],
    metadata: bacnet_router_core::UdpDatagramMeta,
    received_at: Instant,
    ingress_network: NetworkNumber,
    egress_capabilities: PortCapabilities,
    scratch: &mut [u8],
    egress: &mut impl Egress,
) -> Result<(BipIngressContext<'a>, ForwardOutcome), CompositionError> {
    // Keep this boundary open to the router-core's future extended MS/TP
    // profile while rejecting B/IP and opaque application links.  The
    // concrete `MstpEgress` still applies its own profile ceiling and
    // fail-closed peer-capability policy before queue mutation.
    if matches!(
        egress_capabilities.kind,
        bacnet_router_core::PortKind::Bip | bacnet_router_core::PortKind::Other
    ) {
        return Err(CompositionError::InvalidBipPort);
    }
    let context =
        decode_ingress(datagram, metadata, received_at).map_err(CompositionError::BipDecode)?;
    let outcome = router
        .forward_unicast(
            DirectForwardRequest {
                ingress_network,
                logical_source: context.ingress.logical_source_mac(),
                npdu: context.ingress.npdu,
                egress_capabilities,
                now: received_at.as_micros(),
            },
            scratch,
            egress,
        )
        .map_err(CompositionError::Forward)?;
    Ok((context, outcome))
}

/// Route one validated MS/TP delivery action to a B/IP egress.
///
/// A request-bearing delivery reserves a bounded generation binding before
/// forwarding.  If routing or queue admission fails, the tentative binding is
/// removed, so the adapter never hides a failed packet in an application
/// backlog.  A successful `Forwarded` result leaves the request ID available
/// to [`RequestBindings::complete`] or [`RequestBindings::timeout`].
#[allow(clippy::too_many_arguments)]
pub fn forward_mstp_action_to_bip<
    const DIRECT: usize,
    const LEARNED: usize,
    const CAPACITY: usize,
>(
    router: &mut RouterCore<DIRECT, LEARNED>,
    action: Action<'_>,
    ingress_network: NetworkNumber,
    egress_capabilities: PortCapabilities,
    scratch: &mut [u8],
    egress: &mut impl Egress,
    bindings: &mut RequestBindings<CAPACITY>,
    now: u64,
) -> Result<ForwardOutcome, CompositionError> {
    if egress_capabilities.kind != bacnet_router_core::PortKind::Bip {
        return Err(CompositionError::InvalidBipPort);
    }
    let Action::DeliverNpdu {
        source,
        npdu,
        request_id,
        ..
    } = action
    else {
        return Err(CompositionError::InvalidAction);
    };
    if let Some(id) = request_id {
        let admission = bindings.bind(id, now);
        if admission != BindingAdmission::Bound {
            return Err(CompositionError::Binding(admission));
        }
    }
    let logical_source = LinkAddress::try_from_slice(&[source])
        .map_err(|_| CompositionError::Forward(ForwardError::AddressTooLong))?;
    let result = router.forward_unicast(
        DirectForwardRequest {
            ingress_network,
            logical_source,
            npdu,
            egress_capabilities,
            now,
        },
        scratch,
        egress,
    );
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Some(id) = request_id {
                bindings.unbind(id);
            }
            return Err(CompositionError::Forward(error));
        }
    };
    if !matches!(outcome, ForwardOutcome::Forwarded { .. }) {
        if let Some(id) = request_id {
            bindings.unbind(id);
        }
    }
    Ok(outcome)
}

/// Convenience constructor for a B/IP egress used by composition callers.
///
/// The function documents the required port-kind check at the composition
/// boundary while leaving ownership and timestamp behavior to [`BipEgress`].
pub fn bip_egress<'sink, 'scratch, S>(
    sink: &'sink mut S,
    scratch: &'scratch mut [u8],
    egress_capabilities: PortCapabilities,
    broadcast_destination: Ipv4SocketAddress,
    now: Instant,
) -> Result<BipEgress<'sink, 'scratch, S>, CompositionError>
where
    S: crate::bip::BipDatagramSink,
{
    if egress_capabilities.kind != bacnet_router_core::PortKind::Bip {
        return Err(CompositionError::InvalidBipPort);
    }
    Ok(BipEgress::new(
        sink,
        scratch,
        egress_capabilities.id,
        broadcast_destination,
        now,
    ))
}
