//! The network-layer messages a router reads and sends (Clause 6.4).
//!
//! [`code`] has their message types and [`reason`] the reject reasons, the
//! values `bacnet-types` gives them. The views below read a message's data,
//! the payload of an [`Npdu`](crate::npdu::Npdu) whose message type names
//! it, in place: each parses the whole payload or refuses it, so a router
//! never acts on part of a malformed message. The writers fill the caller's
//! buffer and return the length, or `None` when it doesn't fit.
//!
//! The views check layout only. Policy, such as which network numbers a
//! router may learn, stays with the router.

/// Message types (Clause 6.2.4), grouped by what they are for.
pub mod code {
    // Finding routers and the networks behind them.
    /// Asks which router reaches a network, or every network.
    pub const WHO_IS_ROUTER_TO_NETWORK: u8 = 0x00;
    /// Lists networks reachable through the sender.
    pub const I_AM_ROUTER_TO_NETWORK: u8 = 0x01;
    /// Offers a network reachable through a connection not yet open.
    pub const I_COULD_BE_ROUTER_TO_NETWORK: u8 = 0x02;

    // Refusal and flow control.
    /// Tells the originator why a message was not routed.
    pub const REJECT_MESSAGE_TO_NETWORK: u8 = 0x03;
    /// Asks that traffic for some networks be held back.
    pub const ROUTER_BUSY_TO_NETWORK: u8 = 0x04;
    /// Lets that traffic go again.
    pub const ROUTER_AVAILABLE_TO_NETWORK: u8 = 0x05;

    // Routing-table management.
    /// Updates or queries a router's table.
    pub const INITIALIZE_ROUTING_TABLE: u8 = 0x06;
    /// Acknowledges an update, or carries a queried table.
    pub const INITIALIZE_ROUTING_TABLE_ACK: u8 = 0x07;

    // Point-to-point connections, which this stack doesn't open.
    /// Asks a half-router to open a connection.
    pub const ESTABLISH_CONNECTION_TO_NETWORK: u8 = 0x08;
    /// Asks a router to close one.
    pub const DISCONNECT_CONNECTION_TO_NETWORK: u8 = 0x09;

    // Network numbers.
    /// Asks for the local network's number.
    pub const WHAT_IS_NETWORK_NUMBER: u8 = 0x12;
    /// Announces the local network's number.
    pub const NETWORK_NUMBER_IS: u8 = 0x13;

    /// The first of the types the 2020 edition removed (the old security
    /// messages); [`SECURITY_LAST`] is the last.
    pub const SECURITY_FIRST: u8 = 0x0A;
    /// The last of the removed security types.
    pub const SECURITY_LAST: u8 = 0x11;
}

/// Reject reasons (Clause 6.4.4).
pub mod reason {
    /// Some other error.
    pub const OTHER: u8 = 0;
    /// No route to the network, after looking for one.
    pub const NOT_DIRECTLY_CONNECTED: u8 = 1;
    /// The router is busy for that network for now.
    pub const ROUTER_BUSY: u8 = 2;
    /// A message type the router doesn't know.
    pub const UNKNOWN_MESSAGE_TYPE: u8 = 3;
    /// Too long to route to that network.
    pub const MESSAGE_TOO_LONG: u8 = 4;
    /// A DLEN or SLEN no data link uses.
    pub const ADDRESSING_ERROR: u8 = 6;
}

/// A list of two-octet network numbers: the data of I-Am-Router,
/// Router-Busy and Router-Available, and a scoped Who-Is-Router's one
/// network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Networks<'a> {
    octets: &'a [u8],
}

impl<'a> Networks<'a> {
    /// The empty list: for flow control, every network the sender serves.
    pub const EMPTY: Networks<'static> = Networks { octets: &[] };

    /// The list in `payload`, or `None` for an odd length, which ends inside
    /// a network number.
    pub const fn parse(payload: &'a [u8]) -> Option<Self> {
        if payload.len().is_multiple_of(2) {
            Some(Self { octets: payload })
        } else {
            None
        }
    }

    /// How many networks it lists.
    pub const fn len(&self) -> usize {
        self.octets.len() / 2
    }

    /// Whether it lists none.
    pub const fn is_empty(&self) -> bool {
        self.octets.is_empty()
    }

    /// The octets, as received.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.octets
    }

    /// Whether `network` is listed.
    pub fn contains(&self, network: u16) -> bool {
        self.iter().any(|listed| listed == network)
    }

    /// The networks, in the order listed.
    pub fn iter(&self) -> impl Iterator<Item = u16> + 'a {
        self.octets
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
    }

    /// Write `networks` as a list into `out`, returning its length, or
    /// `None` if they don't all fit.
    pub fn write(networks: impl IntoIterator<Item = u16>, out: &mut [u8]) -> Option<usize> {
        let mut at = 0;
        for network in networks {
            out.get_mut(at..at + 2)?
                .copy_from_slice(&network.to_be_bytes());
            at += 2;
        }
        Some(at)
    }
}

/// What a Who-Is-Router asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhoIsRouter {
    /// Every network reachable through the router (no data).
    All,
    /// One network.
    Network(u16),
}

impl WhoIsRouter {
    /// Read a Who-Is-Router's data: nothing, or one network number. Any
    /// other length is malformed, so a bad tail never widens a scoped
    /// question into one about every network.
    pub const fn parse(payload: &[u8]) -> Option<Self> {
        match *payload {
            [] => Some(Self::All),
            [high, low] => Some(Self::Network(u16::from_be_bytes([high, low]))),
            _ => None,
        }
    }
}

/// A Reject-Message-To-Network's data: a reason and a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reject {
    /// The reason, one of [`reason`]'s.
    pub reason: u8,
    /// The network the refused message was for.
    pub network: u16,
}

impl Reject {
    /// Read exactly a reason octet and a network number.
    pub const fn parse(payload: &[u8]) -> Option<Self> {
        match *payload {
            [reason, high, low] => Some(Self {
                reason,
                network: u16::from_be_bytes([high, low]),
            }),
            _ => None,
        }
    }

    /// The data, as sent.
    pub const fn to_bytes(self) -> [u8; 3] {
        let [high, low] = self.network.to_be_bytes();
        [self.reason, high, low]
    }
}

/// Read three octets as a two-octet network and the octet that follows.
const fn network_and_octet(payload: &[u8]) -> Option<(u16, u8)> {
    match *payload {
        [high, low, octet] => Some((u16::from_be_bytes([high, low]), octet)),
        _ => None,
    }
}

/// An I-Could-Be-Router's data: a network and a performance index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ICouldBeRouter {
    /// The network a connection would reach.
    pub network: u16,
    /// The sender's own rating of that connection; lower is better.
    pub performance_index: u8,
}

impl ICouldBeRouter {
    /// Read exactly a network number and the index.
    pub const fn parse(payload: &[u8]) -> Option<Self> {
        match network_and_octet(payload) {
            Some((network, performance_index)) => Some(Self {
                network,
                performance_index,
            }),
            None => None,
        }
    }
}

/// An Establish-Connection's data: a network and how long to hold the
/// connection without traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstablishConnection {
    /// The network to connect to.
    pub network: u16,
    /// Seconds to hold an idle connection; 0 holds it for good.
    pub termination_time: u8,
}

impl EstablishConnection {
    /// Read exactly a network number and the time.
    pub const fn parse(payload: &[u8]) -> Option<Self> {
        match network_and_octet(payload) {
            Some((network, termination_time)) => Some(Self {
                network,
                termination_time,
            }),
            None => None,
        }
    }
}

/// Read a Disconnect-Connection's data: exactly one network number.
pub const fn parse_disconnect(payload: &[u8]) -> Option<u16> {
    match *payload {
        [high, low] => Some(u16::from_be_bytes([high, low])),
        _ => None,
    }
}

/// A Network-Number-Is's data: a network number and whether it was
/// configured or learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkNumberIs {
    /// The local network's number.
    pub network: u16,
    /// 1 when configured, 0 when learned from another Network-Number-Is.
    pub flag: u8,
}

impl NetworkNumberIs {
    /// The flag of a configured number.
    pub const CONFIGURED: u8 = 1;

    /// Read exactly a network number and the flag.
    pub const fn parse(payload: &[u8]) -> Option<Self> {
        match network_and_octet(payload) {
            Some((network, flag)) => Some(Self { network, flag }),
            None => None,
        }
    }

    /// Whether the number was configured.
    pub const fn is_configured(self) -> bool {
        self.flag == Self::CONFIGURED
    }

    /// The data, as sent.
    pub const fn to_bytes(self) -> [u8; 3] {
        let [high, low] = self.network.to_be_bytes();
        [high, low, self.flag]
    }
}

/// What an Initialize-Routing-Table, or its Ack, carries (Clause 6.4.7): a
/// count, then that many entries of a network, a Port ID and Port Info of
/// a stated length.
///
/// A count of 0 in an Initialize-Routing-Table is a query for the whole
/// table. An Ack to an update carries no data at all, which
/// [`RoutingEntries::parse`] refuses: the caller tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingEntries<'a> {
    count: u8,
    entries: &'a [u8],
}

/// One entry of [`RoutingEntries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingEntry<'a> {
    /// The network.
    pub network: u16,
    /// The sender's Port ID for it; 0 asks that the network's rows go.
    pub port_id: u8,
    /// The Port Info, as received (dial-up details this stack doesn't use).
    pub port_info: &'a [u8],
}

impl<'a> RoutingEntries<'a> {
    /// Octets an entry takes with no Port Info.
    pub const ENTRY_LEN: usize = 4;

    /// Read the count and exactly that many entries, or `None` when the
    /// payload is empty, ends inside an entry, or runs past the last one.
    pub fn parse(payload: &'a [u8]) -> Option<Self> {
        let (&count, entries) = payload.split_first()?;
        let mut rest = entries;
        for _ in 0..count {
            rest = split_entry(rest)?.1;
        }
        if !rest.is_empty() {
            return None;
        }
        Some(Self { count, entries })
    }

    /// The count, which is 0 for a query.
    pub const fn count(&self) -> u8 {
        self.count
    }

    /// The entries, in order.
    pub fn iter(&self) -> impl Iterator<Item = RoutingEntry<'a>> + 'a {
        let mut rest = self.entries;
        core::iter::from_fn(move || {
            let (entry, after) = split_entry(rest)?;
            rest = after;
            Some(entry)
        })
    }

    /// Write a count and an entry with no Port Info for each (network,
    /// Port ID) pair into `out`, returning the length, or `None` when they
    /// don't fit or number more than 255.
    pub fn write(entries: impl IntoIterator<Item = (u16, u8)>, out: &mut [u8]) -> Option<usize> {
        let mut count: u8 = 0;
        let mut at = 1;
        for (network, port_id) in entries {
            count = count.checked_add(1)?;
            let [high, low] = network.to_be_bytes();
            out.get_mut(at..at + Self::ENTRY_LEN)?
                .copy_from_slice(&[high, low, port_id, 0]);
            at += Self::ENTRY_LEN;
        }
        *out.first_mut()? = count;
        Some(at)
    }
}

/// The entry at the start of `octets` and what follows it.
fn split_entry(octets: &[u8]) -> Option<(RoutingEntry<'_>, &[u8])> {
    let [high, low, port_id, info_len, ref rest @ ..] = *octets else {
        return None;
    };
    let port_info = rest.get(..usize::from(info_len))?;
    let entry = RoutingEntry {
        network: u16::from_be_bytes([high, low]),
        port_id,
        port_info,
    };
    Some((entry, &rest[port_info.len()..]))
}
