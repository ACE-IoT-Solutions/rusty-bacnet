#![doc = include_str!("../README.md")]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

// bacnet-encoding and bacnet-services pass their feature on to this crate, so
// with neither on they stop here, before their own check can run.
#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!(
    "bacnet-types needs the `std` or the `alloc` feature \
     (bacnet-encoding and bacnet-services pass theirs on)"
);

// `format!` and `vec!` without std's prelude.
#[macro_use]
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
/// The data links this stack serves use 1-octet (MS/TP), 3-octet (the
/// B/IPv6 VMAC) and 6-octet (B/IP, Ethernet, the B/SC VMAC) MACs, so SmallVec
/// keeps every one of them inline.
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;
