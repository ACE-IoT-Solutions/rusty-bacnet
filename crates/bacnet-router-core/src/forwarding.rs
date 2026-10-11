//! Direct-network NPDU rewrite and synchronous egress admission.

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
            || (request.egress_capabilities.kind == PortKind::MstpStandard
                && parsed.destination_mac == [0xff])
        {
            EgressDestination::Broadcast
        } else {
            let valid_length = match request.egress_capabilities.kind {
                PortKind::MstpStandard => parsed.destination_mac.len() == 1,
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

    /// Forward a unicast NPDU through either a configured direct route or a
    /// learned next-hop route.
    ///
    /// Direct egress removes the routed destination and hop-count fields.
    /// Learned egress retains DNET/DADR, replaces DADR with the learned
    /// router address, and decrements the hop count exactly once. Both paths
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
            || route.network != request.egress_capabilities.network
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
                let final_len =
                    match parsed.rewritten_learned_len(request.logical_source, route.next_hop) {
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
            RouteKind::Learned => parsed.rewrite_learned(
                request.ingress_network,
                request.logical_source,
                route.next_hop,
                scratch,
            ),
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
}

impl<const DIRECT: usize, const LEARNED: usize> Default for RouterCore<DIRECT, LEARNED> {
    fn default() -> Self {
        Self::new()
    }
}

struct ParsedNpdu<'a> {
    control: u8,
    destination_network: u16,
    destination_mac: &'a [u8],
    source_fields: Option<&'a [u8]>,
    hop_count: u8,
    remainder: &'a [u8],
}

impl<'a> ParsedNpdu<'a> {
    fn decode(data: &'a [u8]) -> Result<Self, ForwardError> {
        if data.len() < 2 {
            return Err(ForwardError::Truncated);
        }
        if data[0] != BACNET_PROTOCOL_VERSION {
            return Err(ForwardError::UnsupportedVersion(data[0]));
        }
        let control = data[1];
        if control & DESTINATION_BIT == 0 {
            return Err(ForwardError::MissingDestination);
        }
        let mut offset = 2;
        if data.len() < offset + 3 {
            return Err(ForwardError::Truncated);
        }
        let destination_network = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        let destination_len = data[offset] as usize;
        offset += 1;
        if destination_len > 6 || data.len() < offset + destination_len + 1 {
            return Err(if destination_len > 6 {
                ForwardError::AddressTooLong
            } else {
                ForwardError::Truncated
            });
        }
        let destination_mac = &data[offset..offset + destination_len];
        offset += destination_len;

        let source_fields = if control & SOURCE_BIT != 0 {
            let source_start = offset;
            if data.len() < offset + 3 {
                return Err(ForwardError::Truncated);
            }
            let source_network = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            let source_len = data[offset] as usize;
            offset += 1;
            if source_network == 0 || source_network == u16::MAX || source_len == 0 {
                return Err(ForwardError::InvalidSource);
            }
            if source_len > 6 {
                return Err(ForwardError::AddressTooLong);
            }
            if data.len() < offset + source_len + 1 {
                return Err(ForwardError::Truncated);
            }
            offset += source_len;
            Some(&data[source_start..offset])
        } else {
            None
        };

        // Skip Hop Count. The direct egress removes it with DNET/DADR.
        let hop_count = data[offset];
        if hop_count == 0 {
            return Err(ForwardError::HopExhausted);
        }
        offset += 1;
        if control & NETWORK_MESSAGE_BIT != 0 {
            let Some(&message_type) = data.get(offset) else {
                return Err(ForwardError::InvalidNetworkMessage);
            };
            if message_type >= 0x80 && data.len() < offset + 3 {
                return Err(ForwardError::InvalidNetworkMessage);
            }
        }
        Ok(Self {
            control,
            destination_network,
            destination_mac,
            source_fields,
            hop_count,
            remainder: &data[offset..],
        })
    }

    fn rewrite_direct(
        &self,
        ingress_network: NetworkNumber,
        ingress_source: LinkAddress,
        output: &mut [u8],
    ) -> Result<usize, ForwardError> {
        let needed = self.rewritten_direct_len(ingress_source)?;
        if output.len() < needed {
            return Err(ForwardError::ScratchTooSmall {
                needed,
                available: output.len(),
            });
        }
        output[0] = BACNET_PROTOCOL_VERSION;
        output[1] = (self.control & !DESTINATION_BIT) | SOURCE_BIT;
        let mut offset = 2;
        if let Some(source_fields) = self.source_fields {
            output[offset..offset + source_fields.len()].copy_from_slice(source_fields);
            offset += source_fields.len();
        } else {
            output[offset..offset + 2].copy_from_slice(&ingress_network.get().to_be_bytes());
            offset += 2;
            output[offset] = ingress_source.as_slice().len() as u8;
            offset += 1;
            output[offset..offset + ingress_source.as_slice().len()]
                .copy_from_slice(ingress_source.as_slice());
            offset += ingress_source.as_slice().len();
        }
        output[offset..needed].copy_from_slice(self.remainder);
        Ok(needed)
    }

    fn rewritten_direct_len(&self, ingress_source: LinkAddress) -> Result<usize, ForwardError> {
        if self.source_fields.is_none() && ingress_source.is_empty() {
            return Err(ForwardError::InvalidSource);
        }
        let source_len = self
            .source_fields
            .map_or(3 + ingress_source.as_slice().len(), <[u8]>::len);
        Ok(2 + source_len + self.remainder.len())
    }

    fn rewrite_learned(
        &self,
        ingress_network: NetworkNumber,
        ingress_source: LinkAddress,
        next_hop: LinkAddress,
        output: &mut [u8],
    ) -> Result<usize, ForwardError> {
        let needed = self.rewritten_learned_len(ingress_source, next_hop)?;
        if output.len() < needed {
            return Err(ForwardError::ScratchTooSmall {
                needed,
                available: output.len(),
            });
        }
        output[0] = BACNET_PROTOCOL_VERSION;
        output[1] = self.control | SOURCE_BIT;
        let mut offset = 2;
        output[offset..offset + 2].copy_from_slice(&self.destination_network.to_be_bytes());
        offset += 2;
        output[offset] = next_hop.as_slice().len() as u8;
        offset += 1;
        output[offset..offset + next_hop.as_slice().len()].copy_from_slice(next_hop.as_slice());
        offset += next_hop.as_slice().len();
        if let Some(source_fields) = self.source_fields {
            output[offset..offset + source_fields.len()].copy_from_slice(source_fields);
            offset += source_fields.len();
        } else {
            output[offset..offset + 2].copy_from_slice(&ingress_network.get().to_be_bytes());
            offset += 2;
            output[offset] = ingress_source.as_slice().len() as u8;
            offset += 1;
            output[offset..offset + ingress_source.as_slice().len()]
                .copy_from_slice(ingress_source.as_slice());
            offset += ingress_source.as_slice().len();
        }
        output[offset] = self.hop_count() - 1;
        offset += 1;
        output[offset..needed].copy_from_slice(self.remainder);
        Ok(needed)
    }

    fn rewritten_learned_len(
        &self,
        ingress_source: LinkAddress,
        next_hop: LinkAddress,
    ) -> Result<usize, ForwardError> {
        if self.source_fields.is_none() && ingress_source.is_empty() {
            return Err(ForwardError::InvalidSource);
        }
        let source_len = self
            .source_fields
            .map_or(3 + ingress_source.as_slice().len(), <[u8]>::len);
        Ok(2 + 2 + 1 + next_hop.as_slice().len() + source_len + 1 + self.remainder.len())
    }

    fn hop_count(&self) -> u8 {
        self.hop_count
    }
}

fn valid_egress_address(address: &[u8], kind: PortKind) -> bool {
    match kind {
        PortKind::MstpStandard => address.len() == 1,
        PortKind::Bip => address.len() == 6,
        PortKind::Other => address.len() <= 6,
    }
}

fn direct_destination(
    parsed: &ParsedNpdu<'_>,
    capabilities: PortCapabilities,
) -> Result<EgressDestination, ForwardError> {
    if parsed.destination_mac.is_empty()
        || (capabilities.kind == PortKind::MstpStandard && parsed.destination_mac == [0xff])
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
