//! An allocation-free codec for BACnet/IP BVLL messages on IPv4 (Annex J).
//!
//! The crate covers the wire format and nothing above it: the 4-octet BVLL
//! header, the twelve messages Annex J.2 defines, six-octet B/IP addresses
//! (J.1.2), and the entries of BDTs and FDTs.
//! Sockets, BBMD forwarding, the foreign-device registration lifecycle and
//! which device answers what belong to the layer that uses it.
//!
//! The default build never allocates. [`Bvll`] borrows its NPDU or table
//! from the datagram, tables are read through iterators
//! ([`Entries::iter`]), and [`Bvll::encode_into`] writes into caller
//! storage. Only the optional `alloc` and `bytes` sinks grow on the heap.
//!
//! # Validation
//!
//! Decoding is strict about shape and lenient about values.
//! [`BvllFrame::decode`] requires BVLC type 0x81 and a Length field equal to
//! the datagram's length: the field counts the whole message (J.2), so a
//! truncated datagram and one with trailing octets are both refused.
//! [`BvllFrame::message`] then requires a function J.2 defines and a payload
//! of the length its format clause gives (see [`BvlcFunction::payload_shape`]).
//! It does not judge field values: an undefined result code, a zero
//! time-to-live, any table entry and an empty NPDU all decode, and the
//! receiver decides what to do with them. [`decode`] runs both steps.
//!
//! Function 0x0C is unknown here: the clause that once defined it (J.2.13)
//! is deleted in this edition.
//!
//! Encoding is checked. A [`Bvll`] variant holds exactly its function's
//! fields, so most malformed shapes can't be built; [`Bvll::encoded_len`]
//! also refuses a message longer than the Length field can state and a
//! BVLC-Result with a code J.2.1.1 doesn't define.
//!
//! # Features
//!
//! No feature needs `std`. Each adds a [`BvllSink`] implementation:
//!
//! - `alloc`: for `alloc::vec::Vec<u8>`.
//! - `bytes`: for `bytes::BytesMut`; implies `alloc`.
//! - `heapless`: for `heapless::Vec<u8, N>`.
//!
//! # Example
//!
//! ```
//! use bacnet_bip_codec::{decode, Bvll, BvllError, BvlcFunction};
//!
//! let npdu = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];
//! let message = Bvll::OriginalBroadcastNpdu(&npdu);
//! let mut wire = [0u8; 64];
//! let length = message.encode_into(&mut wire)?;
//! assert_eq!(wire[..4], [0x81, 0x0B, 0x00, 0x0A]);
//!
//! let decoded = decode(&wire[..length])?;
//! assert_eq!(decoded, message);
//! assert_eq!(decoded.function(), BvlcFunction::ORIGINAL_BROADCAST_NPDU);
//! assert_eq!(decoded.npdu(), Some(&npdu[..]));
//! # Ok::<(), BvllError>(())
//! ```

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod error;
mod function;
mod message;
mod sink;
mod table;

pub use error::BvllError;
pub use function::{BvlcFunction, BvlcResultCode, PayloadShape};
pub use message::{decode, Bvll, BvllFrame};
pub use sink::BvllSink;
pub use table::{
    address_from_octets, address_from_slice, address_to_octets, BdtEntries, BdtEntry, Entries,
    EntryIter, FdtEntries, FdtEntry, TableEntry,
};

/// The BVLC Type octet of a B/IP message (Annex J.2).
pub const BVLC_TYPE_BIP: u8 = 0x81;
/// The BVLL header: type, function and the two Length octets.
pub const HEADER_LENGTH: usize = 4;
/// The largest message the 16-bit Length field can state, header included.
pub const MAX_BVLL_LENGTH: usize = u16::MAX as usize;
/// Octets in a B/IP address: IPv4 address and UDP port (Annex J.1.2).
pub const BIP_ADDRESS_LENGTH: usize = 6;
/// Octets in a broadcast distribution table entry: B/IP address and mask
/// (Annex J.2.2.1).
pub const BDT_ENTRY_LENGTH: usize = 10;
/// Octets in a foreign device table entry: B/IP address, time-to-live and
/// seconds remaining (Annex J.2.8.1).
pub const FDT_ENTRY_LENGTH: usize = 10;
/// The default B/IP UDP port, 47808 (0xBAC0; Annex J.1.2).
pub const DEFAULT_PORT: u16 = 0xBAC0;
