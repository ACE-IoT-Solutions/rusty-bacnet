#![doc = include_str!("../README.md")]

// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("bacnet-encoding needs the `std` or the `alloc` feature");

// `format!` and `vec!` without std's prelude.
#[macro_use]
extern crate alloc;

pub mod apdu;
pub mod constructed;
pub mod npdu;
pub mod primitives;
pub mod segmentation;
pub mod sink;
pub mod tags;

#[cfg(test)]
mod tags_canonical_tests;
