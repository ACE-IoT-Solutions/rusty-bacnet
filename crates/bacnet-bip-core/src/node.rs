//! A sans-IO B/IP node (Annex J) in normal, foreign-device and BBMD mode.
//!
//! Two layers:
//!
//! - [`Link`] is the fixed configuration: the node's own B/IP address, its
//!   [`LocalSubnet`](crate::LocalSubnet), its [`Mode`] and the largest
//!   datagram it sends. Its decisions are pure. [`Link::receive`] applies
//!   every receive rule to one datagram and says what to do with it,
//!   borrowing the caller's buffer; [`Link::unicast`] and
//!   [`Link::broadcast`] say what to send for an NPDU. A host can share
//!   one copy and decide without a lock.
//! - [`Node`] is the mutable part: this node's own management exchanges,
//!   matched by peer and operation since a BVLC-Result carries no invoke
//!   ID; a foreign device's registration lifecycle and the broadcasts it
//!   holds before its first attempt resolves; and the timers. After every
//!   call the caller polls [`Node::poll_action`] until [`Action::Idle`].

mod actions;
mod addressing;
mod config;
mod counters;
mod engine;
mod exchange;
mod receive;
mod registration;
mod send;

pub use actions::Action;
pub use addressing::{Addressed, Groups, LocalAddresses};
pub use config::{
    ConfigError, Foreign, ForwardedTrust, Link, Mode, Retry, DEFAULT_RESPONSE_TIMEOUT,
    MAX_DATAGRAM, MAX_NPDU, MIN_BUDGET, MIN_RENEWAL,
};
pub use counters::{ExchangeCounters, NodeCounters, RegisterAttempt};
pub use engine::{Inbound, Node};
pub use exchange::{ExchangeId, ExchangeRefusal, LocalFailure, Operation};
pub use receive::{
    Datagram, Delivery, Discard, ForBbmd, Path, Permissive, Policy, Received, Response,
};
pub use registration::{Admission, AttemptOutcome, Lease, RegistrationEvent, RegistrationView};
pub use send::{Cast, Outgoing, SendRefusal};
