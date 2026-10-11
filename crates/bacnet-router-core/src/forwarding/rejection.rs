//! Source-aware refusal actions for routed NPDUs.

use super::{npdu::ParsedNpdu, DirectForwardRequest, ForwardError, ForwardOutcome};
pub use crate::control::RejectReason as ForwardRejectReason;
use crate::{LinkAddress, NetworkNumber};

const BACNET_PROTOCOL_VERSION: u8 = 1;
const NETWORK_MESSAGE: u8 = 0x80;
const DESTINATION_AND_NETWORK_MESSAGE: u8 = 0xa0;
const REJECT_MESSAGE_TO_NETWORK: u8 = 0x03;
const DEFAULT_REJECTION_HOP_COUNT: u8 = 0xff;

/// The original source to which a refusal must be returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalSource {
    /// The incoming NPDU carried SNET/SADR; preserve that origin exactly.
    Routed {
        /// Original BACnet network of the sender.
        network: NetworkNumber,
        /// Original data-link address of the sender.
        address: LinkAddress,
    },
    /// No SNET/SADR was present; use the attributed ingress source.
    Ingress {
        /// BACnet network on which the request arrived.
        network: NetworkNumber,
        /// Logical source attributed by the ingress adapter.
        address: LinkAddress,
    },
}

impl RefusalSource {
    /// Return the BACnet destination network for a refusal NPDU.
    pub const fn network(self) -> NetworkNumber {
        match self {
            Self::Routed { network, .. } | Self::Ingress { network, .. } => network,
        }
    }

    /// Return the data-link destination for a refusal NPDU.
    pub const fn address(self) -> LinkAddress {
        match self {
            Self::Routed { address, .. } | Self::Ingress { address, .. } => address,
        }
    }
}

/// A fully classified Reject-Message-To-Network action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RejectMessage {
    /// Standard network-layer rejection reason.
    pub reason: ForwardRejectReason,
    /// Network named by the rejected request, retained as a raw wire value so
    /// addressing errors can report reserved values as well.
    pub network: u16,
    /// Original source return path.
    pub source: RefusalSource,
}

impl RejectMessage {
    /// Complete NPDU length for this action. Routed returns include a default
    /// hop count; ingress-local returns use a hopless network-message NPDU.
    pub fn encoded_len(self) -> usize {
        match self.source {
            RefusalSource::Routed { address, .. } => {
                2 + 2 + 1 + address.as_slice().len() + 1 + 1 + 1 + 2
            }
            RefusalSource::Ingress { .. } => 2 + 1 + 1 + 2,
        }
    }

    /// Encode the complete routed NPDU that returns this refusal to its
    /// original source. No caller buffer is modified on failure.
    pub fn encode_npdu(self, output: &mut [u8]) -> Result<usize, ForwardError> {
        let needed = self.encoded_len();
        if output.len() < needed {
            return Err(ForwardError::ScratchTooSmall {
                needed,
                available: output.len(),
            });
        }
        let mut offset = 0;
        output[offset] = BACNET_PROTOCOL_VERSION;
        offset += 1;
        match self.source {
            RefusalSource::Routed { network, address } => {
                output[offset] = DESTINATION_AND_NETWORK_MESSAGE;
                offset += 1;
                output[offset..offset + 2].copy_from_slice(&network.get().to_be_bytes());
                offset += 2;
                output[offset] = address.as_slice().len() as u8;
                offset += 1;
                output[offset..offset + address.as_slice().len()]
                    .copy_from_slice(address.as_slice());
                offset += address.as_slice().len();
                output[offset] = DEFAULT_REJECTION_HOP_COUNT;
                offset += 1;
            }
            RefusalSource::Ingress { .. } => {
                // The caller sends this NPDU directly to `source.address()`
                // on the ingress data link. Including DNET/DADR would make a
                // non-router endpoint treat it as a routed packet and drop it.
                output[offset] = NETWORK_MESSAGE;
                offset += 1;
            }
        }
        output[offset] = REJECT_MESSAGE_TO_NETWORK;
        offset += 1;
        output[offset] = self.reason as u8;
        offset += 1;
        output[offset..offset + 2].copy_from_slice(&self.network.to_be_bytes());
        Ok(needed)
    }
}

/// Result of a forwarding attempt together with an optional refusal action.
///
/// The ordinary forwarding methods retain their historical `Result` API. The
/// `*_with_rejection` methods use this wrapper so callers can send a precise
/// refusal without re-parsing the input or guessing the original source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForwardAttempt {
    /// Original forwarding result, including malformed-input errors.
    pub result: Result<ForwardOutcome, ForwardError>,
    /// A refusal action when the input was valid enough to identify its source
    /// and a network-layer rejection is appropriate.
    pub rejection: Option<RejectMessage>,
}

pub(super) fn classify(
    request: DirectForwardRequest<'_>,
    result: &Result<ForwardOutcome, ForwardError>,
) -> Option<RejectMessage> {
    let parsed = ParsedNpdu::decode_for_refusal(request.npdu).ok()?;
    let source = parsed.refusal_source(request.ingress_network, request.logical_source)?;
    let (reason, network) = match result {
        Ok(ForwardOutcome::Oversize { .. }) => (
            ForwardRejectReason::MessageTooLong,
            parsed.destination_network,
        ),
        Ok(ForwardOutcome::QueueFull | ForwardOutcome::PortDown | ForwardOutcome::Expired) => {
            (ForwardRejectReason::RouterBusy, parsed.destination_network)
        }
        Ok(ForwardOutcome::EgressRejected) => (
            ForwardRejectReason::AddressingError,
            parsed.destination_network,
        ),
        Ok(ForwardOutcome::Forwarded { .. }) => return None,
        Err(ForwardError::InvalidDestinationNetwork(network)) => {
            (ForwardRejectReason::AddressingError, *network)
        }
        Err(ForwardError::NoDirectRoute | ForwardError::NoRoute) => (
            ForwardRejectReason::NotDirectlyConnected,
            parsed.destination_network,
        ),
        Err(ForwardError::RouteUnavailable) => {
            (ForwardRejectReason::RouterBusy, parsed.destination_network)
        }
        Err(
            ForwardError::AddressTooLong
            | ForwardError::InvalidSource
            | ForwardError::PortCapabilityMismatch
            | ForwardError::RouteLoopsToIngress
            | ForwardError::InvalidEgressAddress,
        ) => (
            ForwardRejectReason::AddressingError,
            parsed.destination_network,
        ),
        Err(ForwardError::InvalidNetworkMessage) => (
            ForwardRejectReason::UnknownMessageType,
            parsed.destination_network,
        ),
        Err(ForwardError::ScratchTooSmall { .. }) => (
            ForwardRejectReason::MessageTooLong,
            parsed.destination_network,
        ),
        // An exhausted-hop NPDU and a broadcast presented to the unicast
        // seam are consumed without amplifying them with another network
        // message. Broadcast routing belongs to the explicit bounded policy.
        Err(ForwardError::HopExhausted | ForwardError::BroadcastRequiresPolicy) => return None,
        Err(
            ForwardError::Truncated
            | ForwardError::UnsupportedVersion(_)
            | ForwardError::MissingDestination,
        ) => return None,
    };
    Some(RejectMessage {
        reason,
        network,
        source,
    })
}
