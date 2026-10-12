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
//! - [`bbmd`]: a BBMD's two tables (the BDT and the FDT) and foreign-device
//!   registration (Annex J.4, J.5).
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

pub mod bbmd;
mod time;

pub use time::{Duration, Instant};
