//! Allocation-free adapters between the embedded router and concrete links.
//!
//! The adapter crate deliberately stops at the synchronous data-link boundary:
//! platform code owns UART, DMA, and ESP-IDF event-loop integration while the
//! router owns routing policy.  [`mstp::MstpEgress`] is the first composed
//! boundary and places accepted bytes directly into the MS/TP master core's
//! bounded queue.

#![no_std]
#![forbid(unsafe_code)]

pub mod mstp;
