//! Borrowed BACnet/IPv4 BVLL framing for ESP-IDF and other UDP adapters.

use crate::LinkAddress;

/// BACnet/IPv4 BVLC type octet.
pub const BVLL_TYPE_BACNET_IP: u8 = 0x81;
/// Common BVLL header length.
pub const BVLL_HEADER_LENGTH: usize = 4;
const FORWARDED_ADDRESS_LENGTH: usize = 6;

/// IPv4 address and UDP port used as a BACnet/IP MAC.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Ipv4SocketAddress {
    /// IPv4 octets.
    pub ip: [u8; 4],
    /// UDP port without default-port substitution.
    pub port: u16,
}

/// UDP metadata that the platform adapter must preserve for one datagram.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpDatagramMeta {
    /// Actual immediate UDP sender.
    pub peer: Ipv4SocketAddress,
    /// Actual local IPv4 destination, when supplied by packet metadata.
    pub destination: Option<[u8; 4]>,
    /// Whether the destination address was multicast or broadcast.
    pub destination_is_group: bool,
    /// Whether the socket reported a truncated datagram.
    pub truncated: bool,
}

/// NPDU-carrying BVLC functions admitted by the portable adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BvlcFunction {
    /// Forwarded-NPDU (`0x04`).
    ForwardedNpdu,
    /// Original-Unicast-NPDU (`0x0a`).
    OriginalUnicastNpdu,
    /// Original-Broadcast-NPDU (`0x0b`).
    OriginalBroadcastNpdu,
}

impl BvlcFunction {
    /// Return the wire function octet.
    pub const fn to_raw(self) -> u8 {
        match self {
            Self::ForwardedNpdu => 0x04,
            Self::OriginalUnicastNpdu => 0x0a,
            Self::OriginalBroadcastNpdu => 0x0b,
        }
    }
}

/// Borrowed B/IP ingress preserving both immediate and logical origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BipIngress<'a> {
    /// Admitted BVLC function.
    pub function: BvlcFunction,
    /// Actual UDP sender used for peer policy and diagnostics.
    pub immediate_peer: Ipv4SocketAddress,
    /// Logical NPDU source; differs for Forwarded-NPDU.
    pub logical_source: Ipv4SocketAddress,
    /// Actual local destination supplied by the UDP adapter.
    pub local_destination: [u8; 4],
    /// Whether the datagram arrived through a group destination.
    pub link_layer_group: bool,
    /// Borrowed complete NPDU bytes.
    pub npdu: &'a [u8],
}

impl BipIngress<'_> {
    /// Return the logical BACnet/IP source as a six-byte link address.
    ///
    /// For Forwarded-NPDU this is the embedded origin, not the immediate BBMD.
    pub fn logical_source_mac(&self) -> LinkAddress {
        socket_address_mac(self.logical_source)
    }

    /// Return the actual UDP sender as a six-byte link address for peer policy.
    pub fn immediate_peer_mac(&self) -> LinkAddress {
        socket_address_mac(self.immediate_peer)
    }
}

/// Strict portable BVLL ingress failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BipDecodeError {
    /// Platform reported datagram truncation.
    DatagramTruncated,
    /// Destination packet metadata was unavailable.
    MissingDestinationMetadata,
    /// Fewer than four header octets were supplied.
    HeaderTruncated,
    /// The BVLC type was not BACnet/IPv4.
    WrongType(u8),
    /// The declared BVLL length did not exactly equal the UDP datagram length.
    LengthMismatch {
        /// Declared BVLL length.
        declared: usize,
        /// Actual UDP payload length.
        actual: usize,
    },
    /// The BVLC function is not an NPDU-carrying function supported here.
    UnsupportedFunction(u8),
    /// Forwarded-NPDU omitted its embedded origin.
    ForwardedAddressTruncated,
    /// Original unicast/broadcast framing contradicted actual UDP destination class.
    DestinationClassMismatch,
    /// Caller output storage is too small.
    OutputTooSmall {
        /// Required bytes.
        needed: usize,
        /// Available bytes.
        available: usize,
    },
    /// Complete BVLL frame exceeds the 16-bit length field.
    FrameTooLong(usize),
}

/// Strictly decode one complete UDP datagram carrying an NPDU.
pub fn decode_bip_datagram<'a>(
    data: &'a [u8],
    meta: UdpDatagramMeta,
) -> Result<BipIngress<'a>, BipDecodeError> {
    if meta.truncated {
        return Err(BipDecodeError::DatagramTruncated);
    }
    let Some(local_destination) = meta.destination else {
        return Err(BipDecodeError::MissingDestinationMetadata);
    };
    if data.len() < BVLL_HEADER_LENGTH {
        return Err(BipDecodeError::HeaderTruncated);
    }
    if data[0] != BVLL_TYPE_BACNET_IP {
        return Err(BipDecodeError::WrongType(data[0]));
    }
    let declared = u16::from_be_bytes([data[2], data[3]]) as usize;
    if declared != data.len() {
        return Err(BipDecodeError::LengthMismatch {
            declared,
            actual: data.len(),
        });
    }

    let (function, logical_source, npdu) = match data[1] {
        0x04 => {
            if data.len() < BVLL_HEADER_LENGTH + FORWARDED_ADDRESS_LENGTH {
                return Err(BipDecodeError::ForwardedAddressTruncated);
            }
            let origin = Ipv4SocketAddress {
                ip: [data[4], data[5], data[6], data[7]],
                port: u16::from_be_bytes([data[8], data[9]]),
            };
            (
                BvlcFunction::ForwardedNpdu,
                origin,
                &data[BVLL_HEADER_LENGTH + FORWARDED_ADDRESS_LENGTH..],
            )
        }
        0x0a => {
            if meta.destination_is_group {
                return Err(BipDecodeError::DestinationClassMismatch);
            }
            (
                BvlcFunction::OriginalUnicastNpdu,
                meta.peer,
                &data[BVLL_HEADER_LENGTH..],
            )
        }
        0x0b => {
            if !meta.destination_is_group {
                return Err(BipDecodeError::DestinationClassMismatch);
            }
            (
                BvlcFunction::OriginalBroadcastNpdu,
                meta.peer,
                &data[BVLL_HEADER_LENGTH..],
            )
        }
        other => return Err(BipDecodeError::UnsupportedFunction(other)),
    };

    Ok(BipIngress {
        function,
        immediate_peer: meta.peer,
        logical_source,
        local_destination,
        link_layer_group: meta.destination_is_group,
        npdu,
    })
}

/// Encode Original-Unicast or Original-Broadcast into caller storage.
pub fn encode_bvll(
    output: &mut [u8],
    function: BvlcFunction,
    npdu: &[u8],
) -> Result<usize, BipDecodeError> {
    if function == BvlcFunction::ForwardedNpdu {
        return Err(BipDecodeError::UnsupportedFunction(function.to_raw()));
    }
    encode_parts(output, function, &[], npdu)
}

/// Encode Forwarded-NPDU while preserving the original B/IP source.
pub fn encode_forwarded_npdu(
    output: &mut [u8],
    origin: Ipv4SocketAddress,
    npdu: &[u8],
) -> Result<usize, BipDecodeError> {
    let mut address = [0; FORWARDED_ADDRESS_LENGTH];
    address[..4].copy_from_slice(&origin.ip);
    address[4..].copy_from_slice(&origin.port.to_be_bytes());
    encode_parts(output, BvlcFunction::ForwardedNpdu, &address, npdu)
}

fn encode_parts(
    output: &mut [u8],
    function: BvlcFunction,
    prefix: &[u8],
    npdu: &[u8],
) -> Result<usize, BipDecodeError> {
    let total = BVLL_HEADER_LENGTH
        .checked_add(prefix.len())
        .and_then(|length| length.checked_add(npdu.len()))
        .ok_or(BipDecodeError::FrameTooLong(usize::MAX))?;
    if total > u16::MAX as usize {
        return Err(BipDecodeError::FrameTooLong(total));
    }
    if output.len() < total {
        return Err(BipDecodeError::OutputTooSmall {
            needed: total,
            available: output.len(),
        });
    }
    output[0] = BVLL_TYPE_BACNET_IP;
    output[1] = function.to_raw();
    output[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    output[4..4 + prefix.len()].copy_from_slice(prefix);
    output[4 + prefix.len()..total].copy_from_slice(npdu);
    Ok(total)
}

fn socket_address_mac(address: Ipv4SocketAddress) -> LinkAddress {
    let port = address.port.to_be_bytes();
    LinkAddress::try_from_slice(&[
        address.ip[0],
        address.ip[1],
        address.ip[2],
        address.ip[3],
        port[0],
        port[1],
    ])
    .expect("a six-byte B/IP MAC always fits")
}
