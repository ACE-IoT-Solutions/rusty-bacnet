//! A sans-IO B/IP node (Annex J) in normal, foreign-device and BBMD mode.
//!
//! [`Link`] is the fixed configuration: the node's own B/IP address, its
//! [`LocalSubnet`](crate::LocalSubnet), its [`Mode`] and the largest
//! datagram it sends. Its decisions are pure. [`Link::receive`] applies
//! every receive rule to one datagram and says what to do with it,
//! borrowing the caller's buffer; [`Link::unicast`] and [`Link::broadcast`]
//! say what to send for an NPDU. A host can share one copy and decide
//! without a lock.

mod addressing;
mod config;
mod receive;
mod send;

pub use addressing::{Addressed, Groups, LocalAddresses};
pub use config::{
    ConfigError, Foreign, ForwardedTrust, Link, Mode, Retry, DEFAULT_RESPONSE_TIMEOUT,
    MAX_DATAGRAM, MAX_NPDU, MIN_BUDGET, MIN_RENEWAL,
};
pub use receive::{
    Datagram, Delivery, Discard, ForBbmd, Path, Permissive, Policy, Received, Response,
};
pub use send::{Cast, Outgoing, SendRefusal};
