//! Borrowed codecs for the Clause 6 network-control payloads used by the
//! embedded router.
//!
//! The NPDU header (including the network-message-type octet) is deliberately
//! outside this module.  [`decode_control`] takes that octet separately and
//! [`encode_control`] writes only the network-message payload.  This keeps the
//! codec independent of a particular NPDU representation while still making
//! malformed control messages impossible to apply incrementally.

use crate::types::NetworkNumber;

/// Who-Is-Router-To-Network.
pub const WHO_IS_ROUTER_TO_NETWORK: u8 = 0x00;
/// I-Am-Router-To-Network.
pub const I_AM_ROUTER_TO_NETWORK: u8 = 0x01;
/// Reject-Message-To-Network.
pub const REJECT_MESSAGE_TO_NETWORK: u8 = 0x03;
/// Router-Busy-To-Network.
pub const ROUTER_BUSY_TO_NETWORK: u8 = 0x04;
/// Router-Available-To-Network.
pub const ROUTER_AVAILABLE_TO_NETWORK: u8 = 0x05;
/// Initialize-Routing-Table.
pub const INITIALIZE_ROUTING_TABLE: u8 = 0x06;
/// Initialize-Routing-Table-Ack.
pub const INITIALIZE_ROUTING_TABLE_ACK: u8 = 0x07;

/// A borrowed, already validated list of two-octet network numbers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkList<'a> {
    bytes: &'a [u8],
}

impl<'a> NetworkList<'a> {
    /// Number of network numbers in this list.
    pub const fn len(self) -> usize {
        self.bytes.len() / 2
    }

    /// Whether this list contains no network numbers.
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    /// Return the original wire bytes.
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// Iterate over validated network numbers without allocating.
    pub fn iter(self) -> NetworkListIter<'a> {
        NetworkListIter { bytes: self.bytes }
    }
}

/// Iterator over a [`NetworkList`].
#[derive(Clone, Copy, Debug)]
pub struct NetworkListIter<'a> {
    bytes: &'a [u8],
}

impl<'a> Iterator for NetworkListIter<'a> {
    type Item = NetworkNumber;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.len() < 2 {
            return None;
        }
        let network = u16::from_be_bytes([self.bytes[0], self.bytes[1]]);
        self.bytes = &self.bytes[2..];
        // Construction is validated by decode_control, so this cannot fail.
        Some(NetworkNumber::new(network).expect("validated network list"))
    }
}

/// A routing-table entry from Initialize-Routing-Table or its ACK.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoutingTableEntry<'a> {
    /// Reachable BACnet network number.
    pub network: NetworkNumber,
    /// Port identifier assigned by the advertising router.
    pub port_id: u8,
    /// Link-specific port information, borrowed from the wire payload.
    pub port_info: &'a [u8],
}

/// A borrowed, already validated routing-table payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoutingTable<'a> {
    bytes: &'a [u8],
    count: u8,
}

impl<'a> RoutingTable<'a> {
    /// Number of entries in the table.
    pub const fn len(self) -> usize {
        self.count as usize
    }

    /// Whether this table contains no entries.
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// Return the table body, excluding its count octet.
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// Iterate over validated entries without allocating.
    pub fn iter(self) -> RoutingTableIter<'a> {
        RoutingTableIter {
            bytes: self.bytes,
            remaining: self.count,
        }
    }
}

/// Iterator over a [`RoutingTable`].
#[derive(Clone, Copy, Debug)]
pub struct RoutingTableIter<'a> {
    bytes: &'a [u8],
    remaining: u8,
}

impl<'a> Iterator for RoutingTableIter<'a> {
    type Item = RoutingTableEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 || self.bytes.len() < 4 {
            return None;
        }
        let network = u16::from_be_bytes([self.bytes[0], self.bytes[1]]);
        let port_id = self.bytes[2];
        let info_len = self.bytes[3] as usize;
        if self.bytes.len() < 4 + info_len {
            return None;
        }
        let port_info = &self.bytes[4..4 + info_len];
        self.bytes = &self.bytes[4 + info_len..];
        self.remaining -= 1;
        Some(RoutingTableEntry {
            network: NetworkNumber::new(network).expect("validated routing table"),
            port_id,
            port_info,
        })
    }
}

/// Initialize-Routing-Table request body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitializeRoutingTable<'a> {
    /// Query the peer's complete routing table (canonical wire form: empty).
    Query,
    /// Advertise these entries.
    Entries(RoutingTable<'a>),
}

/// A validated network-control message with all variable fields borrowed from
/// the caller's input buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkControl<'a> {
    /// Query one network, or all networks when `network` is `None`.
    WhoIsRouter {
        /// Requested network, or `None` for the all-networks form.
        network: Option<NetworkNumber>,
    },
    /// Announce networks reachable through this router.
    IAmRouter {
        /// Networks announced as reachable through the sender.
        networks: NetworkList<'a>,
    },
    /// Report why a message to `network` was rejected.
    RejectMessageToNetwork {
        /// Standard rejection reason.
        reason: RejectReason,
        /// Destination network whose message was rejected.
        network: NetworkNumber,
    },
    /// Announce temporarily busy networks.
    RouterBusy {
        /// Networks whose learned route is temporarily busy.
        networks: NetworkList<'a>,
    },
    /// Announce networks that are available again.
    RouterAvailable {
        /// Networks whose learned route is available again.
        networks: NetworkList<'a>,
    },
    /// Request a routing table or advertise one.
    InitializeRoutingTable {
        /// Query marker or validated advertised entries.
        table: InitializeRoutingTable<'a>,
    },
    /// Reply with a routing table, including the valid zero-entry form.
    InitializeRoutingTableAck {
        /// Validated routing-table entries.
        table: RoutingTable<'a>,
    },
}

impl NetworkControl<'_> {
    /// Return the Clause 6 network-message type for this control message.
    pub const fn message_type(self) -> u8 {
        match self {
            Self::WhoIsRouter { .. } => WHO_IS_ROUTER_TO_NETWORK,
            Self::IAmRouter { .. } => I_AM_ROUTER_TO_NETWORK,
            Self::RejectMessageToNetwork { .. } => REJECT_MESSAGE_TO_NETWORK,
            Self::RouterBusy { .. } => ROUTER_BUSY_TO_NETWORK,
            Self::RouterAvailable { .. } => ROUTER_AVAILABLE_TO_NETWORK,
            Self::InitializeRoutingTable { .. } => INITIALIZE_ROUTING_TABLE,
            Self::InitializeRoutingTableAck { .. } => INITIALIZE_ROUTING_TABLE_ACK,
        }
    }
}

/// Standard Reject-Message-To-Network reason values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RejectReason {
    /// An implementation-specific reason not covered by another value.
    Other = 0,
    /// The destination network is not directly connected.
    NotDirectlyConnected = 1,
    /// The selected router is temporarily unable to accept the message.
    RouterBusy = 2,
    /// The network-message type is not understood or supported.
    UnknownMessageType = 3,
    /// The message is too long for the next data link.
    MessageTooLong = 4,
    /// The destination address cannot be used on the selected data link.
    AddressingError = 6,
}

impl RejectReason {
    fn from_wire(value: u8) -> Result<Self, ControlCodecError> {
        match value {
            0 => Ok(Self::Other),
            1 => Ok(Self::NotDirectlyConnected),
            2 => Ok(Self::RouterBusy),
            3 => Ok(Self::UnknownMessageType),
            4 => Ok(Self::MessageTooLong),
            6 => Ok(Self::AddressingError),
            value => Err(ControlCodecError::InvalidRejectReason(value)),
        }
    }
}

/// Failure while parsing or encoding a network-control message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlCodecError {
    /// The message type is outside the supported Clause 6 control subset.
    UnsupportedMessageType(u8),
    /// A field was shorter than the wire format requires.
    Truncated {
        /// Byte offset at which the field begins.
        offset: usize,
        /// Minimum complete payload length required at this point.
        needed: usize,
        /// Actual payload length supplied by the caller.
        actual: usize,
    },
    /// The payload has a shape that is not valid for its message type.
    InvalidLength {
        /// Network-message type being decoded.
        message_type: u8,
        /// Invalid payload length.
        length: usize,
    },
    /// Extra bytes remain after all fields have been consumed.
    TrailingBytes {
        /// First unconsumed byte offset.
        offset: usize,
        /// Number of unconsumed bytes.
        remaining: usize,
    },
    /// A reserved network number was present on the wire.
    InvalidNetwork(u16),
    /// A reserved Reject-Message-To-Network reason was present.
    InvalidRejectReason(u8),
    /// The caller's output buffer cannot hold the encoded payload.
    BufferTooSmall {
        /// Exact encoded length required.
        needed: usize,
        /// Bytes available in the caller's buffer.
        capacity: usize,
    },
    /// A port-info slice cannot be represented by the one-octet wire length.
    PortInfoTooLong {
        /// Unrepresentable port-info length.
        length: usize,
    },
}

/// Decode one supported Clause 6 network-control payload.
///
/// Validation is complete before a value is returned. In particular, routing
/// table entries are never exposed one at a time, so a caller cannot mutate a
/// route table after accepting only a prefix of a malformed payload.
pub fn decode_control<'a>(
    message_type: u8,
    payload: &'a [u8],
) -> Result<NetworkControl<'a>, ControlCodecError> {
    match message_type {
        WHO_IS_ROUTER_TO_NETWORK => match payload.len() {
            0 => Ok(NetworkControl::WhoIsRouter { network: None }),
            2 => Ok(NetworkControl::WhoIsRouter {
                network: Some(parse_network(payload, 0)?),
            }),
            length => Err(ControlCodecError::InvalidLength {
                message_type,
                length,
            }),
        },
        I_AM_ROUTER_TO_NETWORK => {
            if payload.is_empty() {
                return Err(ControlCodecError::InvalidLength {
                    message_type,
                    length: 0,
                });
            }
            Ok(NetworkControl::IAmRouter {
                networks: parse_network_list(message_type, payload)?,
            })
        }
        REJECT_MESSAGE_TO_NETWORK => {
            if payload.len() != 3 {
                return Err(ControlCodecError::InvalidLength {
                    message_type,
                    length: payload.len(),
                });
            }
            Ok(NetworkControl::RejectMessageToNetwork {
                reason: RejectReason::from_wire(payload[0])?,
                network: parse_network(payload, 1)?,
            })
        }
        ROUTER_BUSY_TO_NETWORK => Ok(NetworkControl::RouterBusy {
            networks: parse_network_list(message_type, payload)?,
        }),
        ROUTER_AVAILABLE_TO_NETWORK => Ok(NetworkControl::RouterAvailable {
            networks: parse_network_list(message_type, payload)?,
        }),
        INITIALIZE_ROUTING_TABLE => {
            if payload.is_empty() {
                return Ok(NetworkControl::InitializeRoutingTable {
                    table: InitializeRoutingTable::Query,
                });
            }
            let table = parse_routing_table(payload, message_type)?;
            Ok(NetworkControl::InitializeRoutingTable {
                table: if table.is_empty() {
                    // Zero entries is the alternate query spelling used by
                    // existing hosted peers; normalize it to Query.
                    InitializeRoutingTable::Query
                } else {
                    InitializeRoutingTable::Entries(table)
                },
            })
        }
        INITIALIZE_ROUTING_TABLE_ACK => Ok(NetworkControl::InitializeRoutingTableAck {
            table: parse_routing_table(payload, message_type)?,
        }),
        other => Err(ControlCodecError::UnsupportedMessageType(other)),
    }
}

/// Encode a supported network-control payload into a caller-provided buffer.
///
/// The returned length is the number of bytes written. The network-message
/// type is available through [`NetworkControl::message_type`] and is not
/// written here because NPDU encoding owns that field.
pub fn encode_control(
    control: &NetworkControl<'_>,
    out: &mut [u8],
) -> Result<usize, ControlCodecError> {
    let needed = encoded_len(control)?;
    if out.len() < needed {
        return Err(ControlCodecError::BufferTooSmall {
            needed,
            capacity: out.len(),
        });
    }
    let mut offset = 0;
    match *control {
        NetworkControl::WhoIsRouter { network } => {
            if let Some(network) = network {
                put_u16(out, &mut offset, network.get());
            }
        }
        NetworkControl::IAmRouter { networks }
        | NetworkControl::RouterBusy { networks }
        | NetworkControl::RouterAvailable { networks } => {
            out[..networks.as_bytes().len()].copy_from_slice(networks.as_bytes());
            offset = networks.as_bytes().len();
        }
        NetworkControl::RejectMessageToNetwork { reason, network } => {
            out[offset] = reason as u8;
            offset += 1;
            put_u16(out, &mut offset, network.get());
        }
        NetworkControl::InitializeRoutingTable { table } => match table {
            InitializeRoutingTable::Query => {}
            InitializeRoutingTable::Entries(table) => {
                out[offset] = table.len() as u8;
                offset += 1;
                out[offset..offset + table.as_bytes().len()].copy_from_slice(table.as_bytes());
                offset += table.as_bytes().len();
            }
        },
        NetworkControl::InitializeRoutingTableAck { table } => {
            out[offset] = table.len() as u8;
            offset += 1;
            out[offset..offset + table.as_bytes().len()].copy_from_slice(table.as_bytes());
            offset += table.as_bytes().len();
        }
    }
    Ok(offset)
}

/// Return the exact payload length required by [`encode_control`].
pub fn encoded_len(control: &NetworkControl<'_>) -> Result<usize, ControlCodecError> {
    match *control {
        NetworkControl::WhoIsRouter { network } => Ok(if network.is_some() { 2 } else { 0 }),
        NetworkControl::IAmRouter { networks }
        | NetworkControl::RouterBusy { networks }
        | NetworkControl::RouterAvailable { networks } => Ok(networks.as_bytes().len()),
        NetworkControl::RejectMessageToNetwork { reason, .. } => {
            RejectReason::from_wire(reason as u8)?;
            Ok(3)
        }
        NetworkControl::InitializeRoutingTable { table } => Ok(match table {
            InitializeRoutingTable::Query => 0,
            InitializeRoutingTable::Entries(table) => 1 + table.as_bytes().len(),
        }),
        NetworkControl::InitializeRoutingTableAck { table } => Ok(1 + table.as_bytes().len()),
    }
}

fn put_u16(out: &mut [u8], offset: &mut usize, value: u16) {
    let bytes = value.to_be_bytes();
    out[*offset..*offset + 2].copy_from_slice(&bytes);
    *offset += 2;
}

fn parse_network(payload: &[u8], offset: usize) -> Result<NetworkNumber, ControlCodecError> {
    if payload.len() < offset + 2 {
        return Err(ControlCodecError::Truncated {
            offset,
            needed: offset + 2,
            actual: payload.len(),
        });
    }
    let value = u16::from_be_bytes([payload[offset], payload[offset + 1]]);
    NetworkNumber::new(value).map_err(|_| ControlCodecError::InvalidNetwork(value))
}

fn parse_network_list<'a>(
    message_type: u8,
    payload: &'a [u8],
) -> Result<NetworkList<'a>, ControlCodecError> {
    if payload.len() & 1 != 0 {
        return Err(ControlCodecError::InvalidLength {
            message_type,
            length: payload.len(),
        });
    }
    let mut offset = 0;
    while offset < payload.len() {
        parse_network(payload, offset)?;
        offset += 2;
    }
    Ok(NetworkList { bytes: payload })
}

fn parse_routing_table<'a>(
    payload: &'a [u8],
    message_type: u8,
) -> Result<RoutingTable<'a>, ControlCodecError> {
    if payload.is_empty() {
        return Err(ControlCodecError::Truncated {
            offset: 0,
            needed: 1,
            actual: 0,
        });
    }
    let count = payload[0];
    let body = &payload[1..];
    let mut offset = 0;
    for _ in 0..count {
        if body.len() < offset + 4 {
            return Err(ControlCodecError::Truncated {
                offset: offset + 1,
                needed: offset + 5,
                actual: payload.len(),
            });
        }
        parse_network(body, offset)?;
        let info_len = body[offset + 3] as usize;
        let entry_end = offset + 4 + info_len;
        if body.len() < entry_end {
            return Err(ControlCodecError::Truncated {
                offset: offset + 1,
                needed: entry_end + 1,
                actual: payload.len(),
            });
        }
        offset = entry_end;
    }
    if offset != body.len() {
        return Err(ControlCodecError::TrailingBytes {
            offset: offset + 1,
            remaining: body.len() - offset,
        });
    }
    let _ = message_type;
    Ok(RoutingTable { bytes: body, count })
}
