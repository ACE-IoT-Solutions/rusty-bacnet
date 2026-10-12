//! The IP subnet a node is on.

use core::net::{Ipv4Addr, SocketAddrV4};

/// A node's own IP subnet as its B/IP network uses it: the broadcast
/// address that reaches the network's devices there and, when the caller
/// knows it, the subnet mask.
///
/// A BBMD passes it to the decisions that can send to its subnet,
/// [`Tables::forwarded_npdu`](crate::bbmd::Tables::forwarded_npdu) and
/// [`Tables::distribute_broadcast`](crate::bbmd::Tables::distribute_broadcast).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSubnet {
    /// Where a broadcast to the network's devices goes: the subnet's
    /// directed broadcast address (or 255.255.255.255) at the B/IP
    /// network's UDP port (Annex J.1.2).
    pub broadcast: SocketAddrV4,
    /// The mask of the subnet the node's own B/IP address is on, when the
    /// caller knows it. With it, a BBMD doesn't broadcast a peer's
    /// Forwarded-NPDU again when the peer sent it from this subnet at the
    /// BBMD's own port: the network's devices there heard it first hand.
    /// Leave it `None` when it isn't known, or when the own address isn't
    /// on the subnet, as behind NAT; such a forward is then broadcast.
    pub mask: Option<Ipv4Addr>,
}
