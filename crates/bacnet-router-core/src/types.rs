//! Small portable types shared by routing and data-link adapters.

/// Largest link-layer address supported by the embedded B/IP↔MS/TP router.
pub const MAX_LINK_ADDRESS_LEN: usize = 6;
/// Largest NPDU carried by standard-frame MS/TP.
pub const STANDARD_MSTP_MAX_NPDU: usize = 501;

/// A BACnet network number valid for routing-table entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NetworkNumber(u16);

impl NetworkNumber {
    /// Construct a routable network number (`1..=65534`).
    pub const fn new(value: u16) -> Result<Self, NetworkNumberError> {
        if value == 0 || value == u16::MAX {
            Err(NetworkNumberError { value })
        } else {
            Ok(Self(value))
        }
    }

    /// Return the wire value.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A reserved network number was supplied where a routable number is required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkNumberError {
    /// Rejected wire value.
    pub value: u16,
}

/// Stable identifier assigned to one configured router port.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortId(pub u8);

/// The data-link behavior of a router port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortKind {
    /// Annex J BACnet/IPv4.
    Bip,
    /// Clause 9 standard-frame MS/TP, with a 501-byte NPDU ceiling.
    MstpStandard,
    /// An application-defined data link.
    Other,
}

/// Capacity and framing properties used before egress admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortCapabilities {
    /// Stable configured port identity.
    pub id: PortId,
    /// Directly connected BACnet network.
    pub network: NetworkNumber,
    /// Data-link kind.
    pub kind: PortKind,
    /// Largest final encoded NPDU accepted by the link queue.
    pub max_npdu_len: usize,
}

impl PortCapabilities {
    /// Return the usable NPDU ceiling after applying link-format hard limits.
    pub const fn effective_max_npdu_len(self) -> usize {
        match self.kind {
            PortKind::MstpStandard if self.max_npdu_len > STANDARD_MSTP_MAX_NPDU => {
                STANDARD_MSTP_MAX_NPDU
            }
            _ => self.max_npdu_len,
        }
    }
}

/// A bounded link-layer address.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LinkAddress {
    bytes: [u8; MAX_LINK_ADDRESS_LEN],
    len: u8,
}

impl LinkAddress {
    /// Copy an address into bounded storage.
    pub fn try_from_slice(value: &[u8]) -> Result<Self, LinkAddressError> {
        if value.len() > MAX_LINK_ADDRESS_LEN {
            return Err(LinkAddressError::TooLong {
                length: value.len(),
                maximum: MAX_LINK_ADDRESS_LEN,
            });
        }
        let mut bytes = [0; MAX_LINK_ADDRESS_LEN];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    /// Return the address bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Whether this is the empty address used for a link broadcast.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Failure to represent a link-layer address in bounded storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkAddressError {
    /// The address exceeds the embedded router's supported link address size.
    TooLong {
        /// Supplied length.
        length: usize,
        /// Maximum supported length.
        maximum: usize,
    },
}

/// Egress link destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgressDestination {
    /// Send to one data-link address.
    Unicast(LinkAddress),
    /// Send to the data-link broadcast address.
    Broadcast,
}

/// Queue class used to reserve capacity for router control traffic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrafficClass {
    /// Routed application or vendor data.
    Data,
    /// BACnet network-layer control traffic.
    NetworkControl,
}

/// A borrowed request offered directly to an egress queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EgressRequest<'a> {
    /// Target port.
    pub port: PortId,
    /// Link-layer destination.
    pub destination: EgressDestination,
    /// Queue class.
    pub class: TrafficClass,
    /// BACnet network priority (`0..=3`).
    pub priority: u8,
    /// Complete final NPDU after router header rewriting.
    pub npdu: &'a [u8],
}

/// Result returned synchronously by the actual data-link queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    /// The egress queue copied or otherwise took ownership of the NPDU bytes.
    AcceptedOwned,
    /// The applicable bounded queue has no available slot or byte budget.
    Full,
    /// The egress cannot carry the final NPDU length.
    Oversize,
    /// The request expired before the egress queue could own it.
    Expired,
    /// The port is unavailable and did not retain the NPDU.
    PortDown,
    /// The request is invalid for the selected adapter or port.
    Invalid,
}

/// Synchronous boundary implemented by a platform's real egress queues.
pub trait Egress {
    /// Try to admit a complete NPDU. `AcceptedOwned` promises that `request.npdu`
    /// may be released as soon as this call returns.
    fn try_admit(&mut self, request: EgressRequest<'_>) -> AdmissionOutcome;
}

/// Detached router counters suitable for diagnostics/control interfaces.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Diagnostics {
    /// NPDUs submitted to the forwarding core.
    pub ingress_npdus: u64,
    /// NPDUs owned by an egress queue.
    pub forwarded_npdus: u64,
    /// NPDUs rejected because the final rewritten form exceeded egress MTU.
    pub drops_oversize: u64,
    /// NPDUs rejected because the real egress queue was full.
    pub drops_full: u64,
    /// NPDUs rejected because the egress port was down.
    pub drops_port_down: u64,
    /// NPDUs that expired before the egress queue owned them.
    pub drops_expired: u64,
    /// Malformed or unsupported NPDUs.
    pub drops_invalid: u64,
    /// NPDUs for which no usable route was installed.
    pub drops_no_route: u64,
}
