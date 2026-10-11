//! A synchronous, platform-neutral BACnet MS/TP master core.
//!
//! The core owns no clock, serial port, executor, lock, or system resource.
//! Callers supply timestamped events and execute the actions returned by
//! [`MasterCore::poll_action`].
//!
//! # The adapter's side
//!
//! [`MasterCore`] describes the calls in full. Two rules matter most:
//!
//! - **A frame starts only on `Ok`.** Call
//!   [`MasterCore::on_transmit_started`] right before enabling the line
//!   driver, and drive the line only if it succeeds. It refuses a start that
//!   comes within T_turnaround of line activity
//!   ([`CoreError::TransmitDeferred`]), and the frame comes back later as a
//!   new [`Action::Transmit`] (Clause 9.5.5.1).
//! - **Echo begins at the confirmed start.** Until then, keep the receiver
//!   on and feed the core every octet and receive error: they are line
//!   activity. A `SetDirection(Receive)` that arrives before you confirmed a
//!   start means the core took the frame back; drop it. From the confirmed
//!   start to [`MasterCore::on_transmit_complete`], what the receiver hears
//!   is the station's own echo, which the core ignores.
//!
//! Line activity never makes the core abandon a frame it is waiting to send
//! on its own: the frame waits for the line to go quiet. Only the evidence
//! its state watches for, or a time limit taken from that state, drops it,
//! and the drop is counted.
//!
//! # A local policy
//!
//! When a NO_TOKEN deadline is serviced after this station's token
//! generation slot has closed, the core opens the window again a cycle of
//! slots later rather than wait for ever, which Clause 9.5.6.7 does not
//! provide for. Each token generated that way is counted in
//! [`MstpCounters::missed_generation_slots`]; see [`MasterCore`].
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
