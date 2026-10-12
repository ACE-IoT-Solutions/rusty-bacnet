//! Portable BACnet/IP behaviour (Annex J), above the wire format of
//! `bacnet-bip-codec`.
//!
//! The crate is `no_std` and needs no allocator: tables live in arrays whose
//! size the caller picks with a const generic, results are written into
//! storage the caller passes, and time is an [`Instant`] the caller supplies
//! with each call. It reads no clock and opens no socket, so the same code
//! runs on firmware and under the Tokio B/IP transport of
//! `bacnet-transport`, which drives it.
//!
//! [`Instant`] and [`Duration`] are `bacnet-time`'s, in nanoseconds,
//! re-exported here: the types `bacnet-mstp-core` takes too, so firmware
//! that runs both feeds them one clock.
//!
//! - [`bbmd`]: a BBMD's two tables (the BDT and the FDT), foreign-device
//!   registration, and where each broadcast is forwarded (Annex J.4, J.5).
//! - [`node`]: a sans-IO B/IP node in normal, foreign-device and BBMD
//!   mode: its receive rules and NAKs, what it sends, its own management
//!   exchanges, and a foreign device's registration lifecycle (Annex J.2,
//!   J.4, J.5).
//! - [`Arrival`]: whether a datagram was addressed to the node or to its
//!   subnet's broadcast address, which the rules above and a node's own
//!   both depend on, and [`LocalSubnet`], the subnet it reaches that way.
//!
//! # Features
//!
//! None. The crate depends only on `bacnet-bip-codec`, with its default
//! features, and `bacnet-time`, and builds on bare-metal targets without an
//! allocator.

#![no_std]
#![forbid(unsafe_code)]

mod arrival;
pub mod bbmd;
pub mod node;
mod subnet;

pub use arrival::Arrival;
pub use bacnet_time::{Duration, Instant};
pub use subnet::LocalSubnet;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
