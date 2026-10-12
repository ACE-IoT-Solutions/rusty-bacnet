//! Portable BACnet router behaviour (Clause 6), for firmware and for the
//! hosted router of `bacnet-network`.
//!
//! The crate is `no_std` and needs no allocator. NPDUs are read in place
//! from the caller's frame and written into the caller's buffer; the
//! routing table and the discovery state live in arrays whose size the
//! caller picks with const generics; and time is an [`Instant`] the caller
//! passes to each call that needs one. It reads no clock and opens no
//! socket, and nothing on a routing decision formats text or divides a
//! 64-bit value. The table and the discovery state keep no timers of their
//! own, so both start as all zero bits and can live in `.bss`.
//!
//! [`Instant`] and [`Duration`] are `bacnet-time`'s, in nanoseconds,
//! re-exported here: the types `bacnet-mstp-core` and `bacnet-bip-core`
//! take too, so firmware that runs a router between their ports feeds all
//! three one clock.
//!
//! - [`npdu`]: a borrowed view of an NPDU's control information, held equal
//!   to `bacnet-encoding`'s decoder field for field by a property test, and
//!   a writer that refuses an NPDU too long for its buffer before writing an
//!   octet.
//! - [`message`]: the network-layer messages a router reads and sends:
//!   their type codes, the reject reasons, and allocation-free views and
//!   writers for their payloads.
//! - [`table`]: the routing table, sorted by network number, with
//!   learning, the convergence modes, busy and unreachable marks, the
//!   reject hold-down and aging.
//! - [`discovery`]: which unknown networks to look for, at most once per
//!   cooldown each, in bounded storage.
//!
//! A port is identified by its index, a [`PortId`]; its MAC is a [`Mac`] of
//! up to [`MAX_MAC_LEN`] octets.
//!
//! # Clause 6 coverage
//!
//! What this part of the core is answerable for, in our words, with the
//! tests that show it. The router engine, which joins these pieces into
//! forwarding, answering and announcing, adds its own rows.
//!
//! | Obligation | Clause | Test |
//! | --- | --- | --- |
//! | The control octet says which NPCI fields follow, and they follow in one fixed order; a zero DNET, a zero or all-ones SNET and an empty SADR make the NPDU invalid | 6.2, 6.2.2, 6.2.2.1 | `tests/npdu_view.rs`: `view_agrees_with_decode_npdu` |
//! | A DLEN or SLEN past any data link's address width is told apart from other faults, carrying the DNET and the source a reason-6 reject needs | 6.2.2.2, 6.4.4 | `tests/npdu_view.rs`: `over_long_addresses_carry_what_a_reject_needs` |
//! | Each network-layer message's payload is read whole or not at all, and written in the layout the clause gives it | 6.4 | `tests/message.rs` |
//! | The table records, for each network it can reach, the port, the next router (none for a directly attached network), and a state that separates a lasting failure from congestion | 6.6.1 | `tests/table.rs` |
//! | A later claim of the route to a network supersedes the earlier one | 6.6.3.2 | `tests/table_claims.rs` |
//! | A reject with reason 1 marks a learned route unreachable, and one with reason 2 marks it busy for a while | 6.6.3.5 | `tests/table_rejects.rs` |
//! | Flow control holds traffic for the networks named, or for every network the sending router serves when none are named, and lets it go again | 6.6.3.6, 6.6.3.7 | `tests/table_busy.rs` |
//! | A routing-table update replaces or adds a network's row, and Port ID 0 deletes it | 6.4.7, 6.6.3.8 | `tests/table.rs`: `management_updates_never_touch_direct_routes` |
//! | A router with no route to a network looks for one before it refuses the message | 6.6.3.1 | `tests/discovery.rs` |
//!
//! Rules of this stack beyond the clause, each tested in the same files:
//! the 18-octet address bound (#1141); direct routes that no learning,
//! reject or management write changes; a 30-second hold-down per network
//! and arrival port on reject-driven changes; hardened convergence, which
//! waits for a second claim before moving a route to another port; and a
//! discovery cooldown, cap and expiry.
//!
//! One rule departs from the clause, for now: a received reject with a
//! reason other than 1 or 2 removes the learned route, where Clause
//! 6.6.3.5 changes the table for those two reasons only. The hosted router
//! does the same today, and the core keeps it for parity until the fix
//! called R2 in #1679.
//!
//! # Features
//!
//! None. The crate depends only on `bacnet-time` and builds on bare-metal
//! targets without an allocator.

#![no_std]
#![forbid(unsafe_code)]

pub mod discovery;
pub mod message;
pub mod npdu;
pub mod table;

pub use bacnet_time::{Duration, Instant};
pub use npdu::{Mac, MacTooLong};

/// The largest NPDU a B/IP or MS/TP port carries, in octets (Table 6-1;
/// MS/TP needs extended frames for it, Clause 9.3).
pub const MAX_NPDU: usize = 1497;

/// The longest DADR, SADR or link MAC the core accepts, in octets.
///
/// It is `bacnet-encoding`'s `NpduAddress::MAX_MAC_LEN` (#1141), so the
/// core refuses exactly the addresses the hosted codec refuses; a test holds
/// the two equal. No data link in Table 6-2 uses more than 7.
pub const MAX_MAC_LEN: usize = 18;

/// A router port, by its index in the router's port list.
///
/// On the wire, in a routing table's Port ID field, port `n` is `n + 1`:
/// Port ID 0 asks the receiver to delete a network's rows (Clause 6.4.7),
/// so it names no port. Index 255 has no wire Port ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub u8);

impl PortId {
    /// This port's Port ID in a routing table on the wire, or `None` for
    /// index 255, which the one-octet field can't name.
    pub const fn wire_id(self) -> Option<u8> {
        self.0.checked_add(1)
    }

    /// The port a wire Port ID names on a router with `port_count` ports:
    /// `None` for Port ID 0, the delete request, and for a Port ID past the
    /// last port.
    pub const fn from_wire_id(id: u8, port_count: usize) -> Option<Self> {
        match id.checked_sub(1) {
            Some(index) if (index as usize) < port_count => Some(Self(index)),
            _ => None,
        }
    }
}

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
