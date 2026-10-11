//! A synchronous, platform-neutral BACnet MS/TP master core.
//!
//! The core owns no clock, serial port, executor, lock, or system resource.
//! Callers supply timestamped events and execute the actions returned by
//! [`MasterCore::poll_action`].
//!
//! # Features
//!
//! No feature needs `std`. Each adds a [`QueueStorage`] backend for the
//! transmit queues:
//!
//! - `heapless`: `HeaplessQueues`, with capacities fixed at compile time.
//! - `alloc`: `AllocQueues`, which reserves its capacity when built.
//!
//! Without either, the caller implements [`QueueStorage`] itself.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod actions;
mod clock;
mod config;
mod counters;
mod queue;
mod state;

pub use actions::{Action, Direction, RequestId, TransmitFailure, TransmitId};
pub use clock::{Duration, Instant, Monotonic, SimClock};
pub use config::{ConfigError, LocalDurationField, LocalLimitField, MstpCoreConfig};
pub use counters::{MstpCounters, QueueCounters};
#[cfg(feature = "alloc")]
pub use queue::AllocQueues;
#[cfg(feature = "heapless")]
pub use queue::HeaplessQueues;
pub use queue::{
    DequeuedNpdu, NetworkPriority, QueueItemInfo, QueueKind, QueueRejectReason, QueueStorage,
};
pub use state::{
    CoreError, MasterCore, MasterSnapshot, MasterState, ReceiveError, ReplyDecision, LATE_DEADLINE,
    MIN_RX_STORAGE,
};
