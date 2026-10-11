//! Direct-network NPDU rewrite and synchronous egress admission.

mod discovery;
mod npdu;
mod rejection;

use discovery::DiscoveryDecision;
#[allow(unused_imports)]
pub use discovery::{DiscoveryAction, DiscoveryRequest, RouteDiscovery};
use npdu::ParsedNpdu;
pub use rejection::{ForwardAttempt, ForwardRejectReason, RefusalSource, RejectMessage};

use crate::{
    AdmissionOutcome, Diagnostics, Egress, EgressDestination, EgressRequest, LinkAddress,
    NetworkNumber, PortCapabilities, PortKind, Reachability, RouteKind, RouteTable, TrafficClass,
};

const BACNET_PROTOCOL_VERSION: u8 = 1;
const NETWORK_MESSAGE_BIT: u8 = 0x80;
const DESTINATION_BIT: u8 = 0x20;
const SOURCE_BIT: u8 = 0x08;

/// Failure to parse or rewrite an NPDU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardError {
    /// NPDU is shorter than the fixed header.
    Truncated,
    /// Protocol version is not one.
    UnsupportedVersion(u8),
    /// NPDU has no remote destination and is not handled by direct forwarding.
    MissingDestination,
    /// Destination network is reserved or global and needs another policy path.
    InvalidDestinationNetwork(u16),
    /// Destination or source address exceeds the supported B/IP↔MS/TP size.
    AddressTooLong,
    /// Source routing fields are structurally invalid.
    InvalidSource,
    /// Routed NPDU hop count is already zero.
    HopExhausted,
    /// Network-message header is incomplete.
    InvalidNetworkMessage,
    /// No direct route exists for the destination network.
    NoDirectRoute,
    /// No configured or learned route exists for the destination network.
    NoRoute,
    /// Route is currently busy or unavailable.
    RouteUnavailable,
    /// Caller scratch cannot hold the rewritten NPDU.
    ScratchTooSmall {
        /// Rewritten NPDU length.
        needed: usize,
        /// Caller-provided capacity.
        available: usize,
    },
    /// Supplied egress capabilities do not describe the selected direct route.
    PortCapabilityMismatch,
    /// Destination would send the NPDU back onto its ingress network.
    RouteLoopsToIngress,
    /// Destination MAC length is invalid for the selected data link.
    InvalidEgressAddress,
}

/// Observable result of one forwarding attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardOutcome {
    /// The real egress queue owns the rewritten NPDU.
    Forwarded {
        /// Complete NPDU length now owned by the egress queue.
        final_npdu_len: usize,
    },
    /// Final NPDU exceeded the configured port capability.
    Oversize {
        /// Complete rewritten NPDU length.
        final_npdu_len: usize,
        /// Configured egress NPDU ceiling.
        maximum: usize,
    },
    /// Real egress queue had no capacity.
    QueueFull,
    /// Egress port was unavailable.
    PortDown,
    /// Request expired before the egress queue owned it.
    Expired,
    /// Egress rejected an invalid adapter/port request.
    EgressRejected,
}

/// Borrowed context required for one direct-network forwarding decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectForwardRequest<'a> {
    /// Directly connected network on which the NPDU arrived.
    pub ingress_network: NetworkNumber,
    /// Logical ingress source used for SADR insertion.
    ///
    /// For B/IP Forwarded-NPDU this must be the embedded origin, not the
    /// immediate BBMD peer. [`crate::BipIngress::logical_source_mac`] provides
    /// the correctly attributed value.
    pub logical_source: LinkAddress,
    /// Complete incoming NPDU.
    pub npdu: &'a [u8],
    /// Capabilities of the direct port selected by configuration.
    pub egress_capabilities: PortCapabilities,
    /// Caller monotonic timestamp.
    pub now: u64,
}

/// Forwarding result with caller-owned route-discovery state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForwardDiscoveryAttempt {
    /// Ordinary forwarding result. Unknown routes remain visible as
    /// [`ForwardError::NoRoute`] or [`ForwardError::NoDirectRoute`].
    pub result: Result<ForwardOutcome, ForwardError>,
    /// Who-Is-Router, pending, or bounded-capacity action for the caller.
    pub discovery: Option<DiscoveryAction>,
    /// Source-aware refusal, available only after discovery timed out (or for
    /// errors that do not require route discovery).
    pub rejection: Option<RejectMessage>,
}

/// Portable router state for the initial direct-forwarding slice.
#[derive(Clone, Debug)]
pub struct RouterCore<const DIRECT: usize, const LEARNED: usize> {
    routes: RouteTable<DIRECT, LEARNED>,
    diagnostics: Diagnostics,
}

impl<const DIRECT: usize, const LEARNED: usize> RouterCore<DIRECT, LEARNED> {
    /// Construct an empty core.
    pub const fn new() -> Self {
        Self {
            routes: RouteTable::new(),
            diagnostics: Diagnostics {
                ingress_npdus: 0,
                forwarded_npdus: 0,
                drops_oversize: 0,
                drops_full: 0,
                drops_port_down: 0,
                drops_expired: 0,
                drops_invalid: 0,
                drops_no_route: 0,
            },
        }
    }

    /// Mutable route-table access for configuration and control processing.
    pub fn routes_mut(&mut self) -> &mut RouteTable<DIRECT, LEARNED> {
        &mut self.routes
    }

    /// Read-only route-table access.
    pub const fn routes(&self) -> &RouteTable<DIRECT, LEARNED> {
        &self.routes
    }

    /// Detached diagnostic snapshot.
    pub const fn diagnostics(&self) -> Diagnostics {
        self.diagnostics
    }

    /// Clear all diagnostic counters without changing routing state.
    pub fn clear_diagnostics(&mut self) {
        self.diagnostics = Diagnostics::default();
    }

    /// Rewrite and synchronously admit an NPDU to a directly connected network.
    ///
    /// The final encoded NPDU length is checked after removing DNET/DADR/Hop
    /// Count and, when absent, adding SNET/SADR for the ingress network.
    pub fn forward_direct(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
    ) -> Result<ForwardOutcome, ForwardError> {
        self.diagnostics.ingress_npdus = self.diagnostics.ingress_npdus.saturating_add(1);
        let parsed = match ParsedNpdu::decode(request.npdu) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(error);
            }
        };
        let destination_network = match NetworkNumber::new(parsed.destination_network) {
            Ok(network) => network,
            Err(_) => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(ForwardError::InvalidDestinationNetwork(
                    parsed.destination_network,
                ));
            }
        };
        let Some(route) = self.routes.lookup(destination_network) else {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::NoDirectRoute);
        };
        if route.kind != RouteKind::Direct {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::NoDirectRoute);
        }
        if destination_network == request.ingress_network {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::RouteLoopsToIngress);
        }
        if route.port != request.egress_capabilities.id
            || destination_network != request.egress_capabilities.network
        {
            self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
            return Err(ForwardError::PortCapabilityMismatch);
        }
        if route.effective_reachability(request.now) != Reachability::Reachable {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::RouteUnavailable);
        }

        let destination = if parsed.destination_mac.is_empty()
            || (matches!(
                request.egress_capabilities.kind,
                PortKind::MstpStandard | PortKind::MstpExtended
            ) && parsed.destination_mac == [0xff])
        {
            EgressDestination::Broadcast
        } else {
            let valid_length = match request.egress_capabilities.kind {
                PortKind::MstpStandard | PortKind::MstpExtended => {
                    parsed.destination_mac.len() == 1
                }
                PortKind::Bip => parsed.destination_mac.len() == 6,
                PortKind::Other => true,
            };
            if !valid_length {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(ForwardError::InvalidEgressAddress);
            }
            EgressDestination::Unicast(
                LinkAddress::try_from_slice(parsed.destination_mac)
                    .map_err(|_| ForwardError::AddressTooLong)?,
            )
        };
        let final_len = match parsed.rewritten_direct_len(request.logical_source) {
            Ok(length) => length,
            Err(error) => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(error);
            }
        };
        let maximum = request.egress_capabilities.effective_max_npdu_len();
        if final_len > maximum {
            self.diagnostics.drops_oversize = self.diagnostics.drops_oversize.saturating_add(1);
            return Ok(ForwardOutcome::Oversize {
                final_npdu_len: final_len,
                maximum,
            });
        }
        if let Err(error) =
            parsed.rewrite_direct(request.ingress_network, request.logical_source, scratch)
        {
            self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
            return Err(error);
        }
        let class = if parsed.control & NETWORK_MESSAGE_BIT != 0 {
            TrafficClass::NetworkControl
        } else {
            TrafficClass::Data
        };
        match egress.try_admit(EgressRequest {
            port: route.port,
            destination,
            class,
            priority: parsed.control & 0x03,
            npdu: &scratch[..final_len],
        }) {
            AdmissionOutcome::AcceptedOwned => {
                self.diagnostics.forwarded_npdus =
                    self.diagnostics.forwarded_npdus.saturating_add(1);
                Ok(ForwardOutcome::Forwarded {
                    final_npdu_len: final_len,
                })
            }
            AdmissionOutcome::Full => {
                self.diagnostics.drops_full = self.diagnostics.drops_full.saturating_add(1);
                Ok(ForwardOutcome::QueueFull)
            }
            AdmissionOutcome::Oversize => {
                self.diagnostics.drops_oversize = self.diagnostics.drops_oversize.saturating_add(1);
                Ok(ForwardOutcome::Oversize {
                    final_npdu_len: final_len,
                    maximum,
                })
            }
            AdmissionOutcome::PortDown => {
                self.diagnostics.drops_port_down =
                    self.diagnostics.drops_port_down.saturating_add(1);
                Ok(ForwardOutcome::PortDown)
            }
            AdmissionOutcome::Expired => {
                self.diagnostics.drops_expired = self.diagnostics.drops_expired.saturating_add(1);
                Ok(ForwardOutcome::Expired)
            }
            AdmissionOutcome::Invalid => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                Ok(ForwardOutcome::EgressRejected)
            }
        }
    }

    /// Rewrite and admit an NPDU while retaining a source-aware refusal
    /// action for every valid request that cannot be forwarded.
    ///
    /// The returned [`RejectMessage`] is caller-buffered and is not admitted
    /// automatically. This keeps refusal routing on the same synchronous
    /// egress boundary as ordinary traffic while allowing the adapter to
    /// choose the correct ingress port and queue.
    pub fn forward_direct_with_rejection(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
    ) -> ForwardAttempt {
        let result = self.forward_direct(request, scratch, egress);
        let rejection = rejection::classify(request, &result);
        ForwardAttempt { result, rejection }
    }

    /// Forward a direct-route request while giving an unknown network one
    /// bounded Who-Is-Router discovery opportunity before refusing it.
    pub fn forward_direct_with_discovery<const CAPACITY: usize>(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
        discovery: &mut RouteDiscovery<CAPACITY>,
    ) -> ForwardDiscoveryAttempt {
        let result = self.forward_direct(request, scratch, egress);
        self.finish_discovery(request, result, discovery, true)
    }

    /// Forward a unicast NPDU through either a configured direct route or a
    /// learned next-hop route.
    ///
    /// Direct egress removes the routed destination and hop-count fields.
    /// Learned egress retains the final DNET/DADR, uses the learned router as
    /// the link-layer next hop, and decrements the hop count exactly once. Both paths
    /// use the same validated NPDU view and preserve/inject SNET/SADR.
    pub fn forward_unicast(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
    ) -> Result<ForwardOutcome, ForwardError> {
        self.diagnostics.ingress_npdus = self.diagnostics.ingress_npdus.saturating_add(1);
        let parsed = match ParsedNpdu::decode(request.npdu) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(error);
            }
        };
        let destination_network = match NetworkNumber::new(parsed.destination_network) {
            Ok(network) => network,
            Err(_) => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                return Err(ForwardError::InvalidDestinationNetwork(
                    parsed.destination_network,
                ));
            }
        };
        let Some(route) = self.routes.lookup(destination_network) else {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::NoRoute);
        };
        if destination_network == request.ingress_network {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::RouteLoopsToIngress);
        }
        if route.port != request.egress_capabilities.id
            || (route.kind == RouteKind::Direct
                && route.network != request.egress_capabilities.network)
        {
            self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
            return Err(ForwardError::PortCapabilityMismatch);
        }
        if route.effective_reachability(request.now) != Reachability::Reachable {
            self.diagnostics.drops_no_route = self.diagnostics.drops_no_route.saturating_add(1);
            return Err(ForwardError::RouteUnavailable);
        }

        let (destination, final_len) = match route.kind {
            RouteKind::Direct => {
                let destination = match direct_destination(&parsed, request.egress_capabilities) {
                    Ok(destination) => destination,
                    Err(error) => {
                        self.diagnostics.drops_invalid =
                            self.diagnostics.drops_invalid.saturating_add(1);
                        return Err(error);
                    }
                };
                let final_len = match parsed.rewritten_direct_len(request.logical_source) {
                    Ok(length) => length,
                    Err(error) => {
                        self.diagnostics.drops_invalid =
                            self.diagnostics.drops_invalid.saturating_add(1);
                        return Err(error);
                    }
                };
                (destination, final_len)
            }
            RouteKind::Learned => {
                if route.next_hop.is_empty()
                    || !valid_egress_address(
                        route.next_hop.as_slice(),
                        request.egress_capabilities.kind,
                    )
                {
                    self.diagnostics.drops_invalid =
                        self.diagnostics.drops_invalid.saturating_add(1);
                    return Err(ForwardError::InvalidEgressAddress);
                }
                let final_len = match parsed.rewritten_learned_len(request.logical_source) {
                    Ok(length) => length,
                    Err(error) => {
                        self.diagnostics.drops_invalid =
                            self.diagnostics.drops_invalid.saturating_add(1);
                        return Err(error);
                    }
                };
                (EgressDestination::Unicast(route.next_hop), final_len)
            }
        };
        let maximum = request.egress_capabilities.effective_max_npdu_len();
        if final_len > maximum {
            self.diagnostics.drops_oversize = self.diagnostics.drops_oversize.saturating_add(1);
            return Ok(ForwardOutcome::Oversize {
                final_npdu_len: final_len,
                maximum,
            });
        }
        let rewrite_result = match route.kind {
            RouteKind::Direct => {
                parsed.rewrite_direct(request.ingress_network, request.logical_source, scratch)
            }
            RouteKind::Learned => {
                parsed.rewrite_learned(request.ingress_network, request.logical_source, scratch)
            }
        };
        if let Err(error) = rewrite_result {
            self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
            return Err(error);
        }
        let class = if parsed.control & NETWORK_MESSAGE_BIT != 0 {
            TrafficClass::NetworkControl
        } else {
            TrafficClass::Data
        };
        match egress.try_admit(EgressRequest {
            port: route.port,
            destination,
            class,
            priority: parsed.control & 0x03,
            npdu: &scratch[..final_len],
        }) {
            AdmissionOutcome::AcceptedOwned => {
                self.diagnostics.forwarded_npdus =
                    self.diagnostics.forwarded_npdus.saturating_add(1);
                Ok(ForwardOutcome::Forwarded {
                    final_npdu_len: final_len,
                })
            }
            AdmissionOutcome::Full => {
                self.diagnostics.drops_full = self.diagnostics.drops_full.saturating_add(1);
                Ok(ForwardOutcome::QueueFull)
            }
            AdmissionOutcome::Oversize => {
                self.diagnostics.drops_oversize = self.diagnostics.drops_oversize.saturating_add(1);
                Ok(ForwardOutcome::Oversize {
                    final_npdu_len: final_len,
                    maximum,
                })
            }
            AdmissionOutcome::PortDown => {
                self.diagnostics.drops_port_down =
                    self.diagnostics.drops_port_down.saturating_add(1);
                Ok(ForwardOutcome::PortDown)
            }
            AdmissionOutcome::Expired => {
                self.diagnostics.drops_expired = self.diagnostics.drops_expired.saturating_add(1);
                Ok(ForwardOutcome::Expired)
            }
            AdmissionOutcome::Invalid => {
                self.diagnostics.drops_invalid = self.diagnostics.drops_invalid.saturating_add(1);
                Ok(ForwardOutcome::EgressRejected)
            }
        }
    }

    /// Learned-route equivalent of [`Self::forward_direct_with_rejection`].
    pub fn forward_unicast_with_rejection(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
    ) -> ForwardAttempt {
        let result = self.forward_unicast(request, scratch, egress);
        let rejection = rejection::classify(request, &result);
        ForwardAttempt { result, rejection }
    }

    /// Forward a unicast request while discovering an unknown destination
    /// network. The original NPDU remains caller-owned; this method only
    /// returns a bounded action and suppresses the immediate reason-1 reject.
    pub fn forward_unicast_with_discovery<const CAPACITY: usize>(
        &mut self,
        request: DirectForwardRequest<'_>,
        scratch: &mut [u8],
        egress: &mut impl Egress,
        discovery: &mut RouteDiscovery<CAPACITY>,
    ) -> ForwardDiscoveryAttempt {
        let result = self.forward_unicast(request, scratch, egress);
        self.finish_discovery(request, result, discovery, false)
    }

    fn finish_discovery<const CAPACITY: usize>(
        &self,
        request: DirectForwardRequest<'_>,
        result: Result<ForwardOutcome, ForwardError>,
        discovery: &mut RouteDiscovery<CAPACITY>,
        direct: bool,
    ) -> ForwardDiscoveryAttempt {
        let unknown_route = if direct {
            matches!(result, Err(ForwardError::NoDirectRoute))
        } else {
            matches!(result, Err(ForwardError::NoRoute))
        };
        if !unknown_route {
            let rejection = rejection::classify(request, &result);
            return ForwardDiscoveryAttempt {
                result,
                discovery: None,
                rejection,
            };
        }

        let Some(network) = ParsedNpdu::decode_for_refusal(request.npdu)
            .ok()
            .and_then(|parsed| NetworkNumber::new(parsed.destination_network).ok())
        else {
            return ForwardDiscoveryAttempt {
                result,
                discovery: None,
                rejection: None,
            };
        };
        match discovery.begin(network, request.now) {
            DiscoveryDecision::Action(action) => ForwardDiscoveryAttempt {
                result,
                discovery: Some(action),
                rejection: None,
            },
            DiscoveryDecision::TimedOut => ForwardDiscoveryAttempt {
                result,
                discovery: None,
                rejection: rejection::classify(request, &result),
            },
        }
    }
}

impl<const DIRECT: usize, const LEARNED: usize> Default for RouterCore<DIRECT, LEARNED> {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_egress_address(address: &[u8], kind: PortKind) -> bool {
    match kind {
        PortKind::MstpStandard | PortKind::MstpExtended => address.len() == 1,
        PortKind::Bip => address.len() == 6,
        PortKind::Other => address.len() <= 6,
    }
}

fn direct_destination(
    parsed: &ParsedNpdu<'_>,
    capabilities: PortCapabilities,
) -> Result<EgressDestination, ForwardError> {
    if parsed.destination_mac.is_empty()
        || (matches!(
            capabilities.kind,
            PortKind::MstpStandard | PortKind::MstpExtended
        ) && parsed.destination_mac == [0xff])
    {
        return Ok(EgressDestination::Broadcast);
    }
    if !valid_egress_address(parsed.destination_mac, capabilities.kind) {
        return Err(ForwardError::InvalidEgressAddress);
    }
    Ok(EgressDestination::Unicast(
        LinkAddress::try_from_slice(parsed.destination_mac)
            .map_err(|_| ForwardError::AddressTooLong)?,
    ))
}
