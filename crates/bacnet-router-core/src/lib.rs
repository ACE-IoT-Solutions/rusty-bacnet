//! A synchronous, bounded BACnet network router core for embedded adapters.
//!
//! This crate owns no sockets, serial ports, clock, executor, or allocator.
//! Platform adapters decode link framing, call the core, and implement
//! [`Egress`] by copying accepted NPDUs into the actual data-link queue.

#![no_std]
#![forbid(unsafe_code)]

mod bip;
pub mod broadcast;
pub mod control;
mod duplicate;
mod forwarding;
mod route_table;
mod router_control;
mod types;

pub use bip::{
    decode_bip_datagram, encode_bvll, encode_forwarded_npdu, BipDecodeError, BipIngress,
    BvlcFunction, Ipv4SocketAddress, UdpDatagramMeta, BVLL_HEADER_LENGTH, BVLL_TYPE_BACNET_IP,
};
pub use broadcast::{
    BroadcastDestination, BroadcastError, BroadcastFanoutOutcome, BroadcastFanoutResult,
    BroadcastPolicy, BroadcastPolicyConfig, BroadcastReport, BroadcastRequest, BroadcastTarget,
};
pub use duplicate::{DuplicateCache, DuplicateCacheStats, DuplicateKey, DuplicateOutcome};
pub use forwarding::{
    DirectForwardRequest, ForwardAttempt, ForwardError, ForwardOutcome, ForwardRejectReason,
    RefusalSource, RejectMessage, RouterCore,
};
pub use route_table::{
    LearnOutcome, Reachability, RouteEntry, RouteKind, RouteTable, RouteTableError,
};
pub use router_control::{
    route_reachability, ControlDelivery, ControlOutputError, NetworkSet, PortIdSemantics,
    RouteSnapshot, RouterControl, RouterControlAction, RouterControlError, RouterControlOutcome,
    RouterControlRequest, DEFAULT_BUSY_DURATION, DEFAULT_LEARNED_AGE,
};
pub use types::{
    AdmissionOutcome, Diagnostics, Egress, EgressDestination, EgressRequest, LinkAddress,
    LinkAddressError, NetworkNumber, NetworkNumberError, PortCapabilities, PortId, PortKind,
    TrafficClass, STANDARD_MSTP_MAX_NPDU,
};
