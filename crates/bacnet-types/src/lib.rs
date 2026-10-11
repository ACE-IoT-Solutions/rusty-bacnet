//! BACnet protocol types per ASHRAE 135-2020.
//!
//! This crate provides the foundational types for the BACnet protocol:
//! enumerations, primitive data types, addresses, and error types.
//!
//! Its runtime dependencies are `bitflags`, `smallvec` and `thiserror`.
//!
//! Every enum newtype implements `FromStr`, accepting any common case style
//! (`analog-input`, `AnalogInput`, `ANALOG_INPUT`, ...) for its named
//! constants. The optional `serde` feature adds `Deserialize` for
//! [`enums::ObjectType`] and [`enums::PropertyIdentifier`] on top of it.
//!
//! # Features
//!
//! The crate needs an allocator, and one of these features to say so:
//!
//! - `std` (default): hosted builds. Adds the transport I/O error
//!   (`Error::Transport`) and the B/IP port contract (`bip_port`).
//! - `alloc`: `no_std` with a global allocator, for bare-metal targets:
//!   `default-features = false, features = ["alloc"]`.
//!
//! A build with neither fails; that combination is reserved for a future
//! allocation-free subset.

// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("bacnet-types needs the `std` or the `alloc` feature");

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
pub mod bip_port;
pub mod bitstring;
pub mod calendar;
pub mod constructed;
pub mod data_link;
pub mod enums;
pub mod error;
pub mod network_number;
pub mod primitives;

/// BACnet MAC address — stack-allocated for typical sizes (≤6 bytes).
///
/// BACnet MAC addresses are 1 byte (MS/TP), 6 bytes (Ethernet/IP),
/// 3 bytes (SC VMAC), or 18 bytes (BIP6 IP:port). SmallVec avoids
/// heap allocation for the common cases.
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;
