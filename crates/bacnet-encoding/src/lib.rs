#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = "BACnet ASN.1/BER encoding and APDU/NPDU codecs per ASHRAE 135-2020."]
#![doc = ""]
#![doc = "The `alloc` feature enables the codec on no_std targets with a global allocator."]
#![doc = "For example, an embedded application can use the primitive encoder directly:"]
#![doc = ""]
#![doc = "```rust"]
#![doc = "extern crate alloc;"]
#![doc = "use alloc::vec::Vec;"]
#![doc = "use bacnet_encoding::primitives::encode_unsigned;"]
#![doc = "use bytes::BytesMut;"]
#![doc = ""]
#![doc = "fn example() {"]
#![doc = "let mut encoded = BytesMut::new();"]
#![doc = "encode_unsigned(&mut encoded, 42);"]
#![doc = "let _wire: Vec<u8> = encoded.to_vec();"]
#![doc = "# }"]
#![doc = "```"]
#![deny(unsafe_code)]

#[cfg(all(not(feature = "std"), not(feature = "alloc")))]
compile_error!("bacnet-encoding requires either the `std` or `alloc` feature");

#[macro_use]
extern crate alloc;

pub mod apdu;
pub mod constructed;
pub mod npdu;
pub mod primitives;
pub mod segmentation;
pub mod tags;

#[cfg(test)]
mod tags_canonical_tests;
