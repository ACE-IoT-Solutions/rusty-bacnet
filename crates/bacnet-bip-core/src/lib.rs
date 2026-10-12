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
//! - [`bbmd`]: a BBMD's two tables (the BDT and the FDT), foreign-device
//!   registration, and where each broadcast is forwarded (Annex J.4, J.5).
//! - [`Arrival`]: whether a datagram was addressed to the node or to its
//!   subnet's broadcast address, which the rules above and a node's own
//!   both depend on, and [`LocalSubnet`], the subnet it reaches that way.
//!
//! A sans-IO engine for a B/IP node in normal and foreign-device mode will
//! join it here.
//!
//! # Features
//!
//! None. The crate depends only on `bacnet-bip-codec`, with its default
//! features, and builds on bare-metal targets without an allocator.

#![no_std]
#![forbid(unsafe_code)]

mod arrival;
pub mod bbmd;
mod subnet;
mod time;

pub use arrival::Arrival;
pub use subnet::LocalSubnet;
pub use time::{Duration, Instant};

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
