//! BACnetServer: builder, APDU dispatch, and lifecycle management.
//!
//! The server wraps a NetworkLayer behind Arc (shared with the dispatch task),
//! owns an ObjectDatabase via Arc<Mutex>, and spawns a dispatch task that
//! routes incoming APDUs to service handlers.

use std::collections::HashMap;
use std::net::Ipv4Addr;
#[cfg(test)]
pub(crate) use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, validate_max_apdu_length, AbortPdu, Apdu, ComplexAck,
    ConfirmedRequest as ConfirmedRequestPdu, ErrorPdu, RejectPdu, SegmentAck as SegmentAckPdu,
    SimpleAck, UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::segmentation::{
    duplicate_in_window, max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_network::observer::ApduObserver;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::{
    lookup_notification_recipients, resolve_transition_priority_ack,
};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, LifeSafetyOperation,
    NetworkPriority, NotifyType, ObjectType, PropertyIdentifier, RejectReason, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

use crate::audit_notification::{
    AuditNotificationAuthorizationContext, AuditNotificationAuthorizer,
    UnconfirmedAuditNotificationAuthorizationContext, UnconfirmedAuditNotificationAuthorizer,
    MAX_AUDIT_NOTIFICATIONS, MAX_AUDIT_NOTIFICATION_BYTES,
};
pub use crate::cov::{CovCounters, CovPolicy};
use crate::cov::{CovNotificationKind, CovSubscription, CovSubscriptionTable};
use crate::handlers;
use crate::life_safety::{LifeSafetyOperationAuthorizationContext, LifeSafetyOperationAuthorizer};
use confirmed_request_tracker::{ConfirmedRequestAdmission, ConfirmedRequestTracker};
pub use device_bindings::DeviceBinding;
use device_bindings::{register_configured_binding, DeviceBindingTable};
use notification_transactions::{
    canonical_direct_peer, canonical_routed_peer, run_notification_worker,
    NotificationTransactions, NotificationWorkerResult,
};

/// Maximum number of concurrent segmented reassembly sessions.
const MAX_SEG_RECEIVERS: usize = 128;

/// Hard per-request reassembly ceiling: the sequence-number space (#364).
///
/// A local storage bound, not a protocol one. Clause 20.1.2.7 makes the
/// request sequence number modulo 256, so a longer request is entirely
/// representable on the wire — this server simply keys its segment store by
/// that `u8` and cannot tell segment 256 from segment 0. A Device object
/// configured to receive segments does publish a tighter advertisement — its
/// `Max_Segments_Accepted` (Clause 12.11) defaults to `Unsigned(65)` — but
/// enforcing it here is deliberately not done: accepting more segments than
/// advertised is permissive, not a violation, while the sequence space is the
/// line past which acceptance silently corrupts. Exactly 256 segments
/// reassemble correctly and must keep working; 257 is the first that would
/// corrupt the payload.
const MAX_REQUEST_SEGMENTS: usize = 256;

/// Maximum number of concurrent segmented response send sessions.
const MAX_SEG_SENDERS: usize = 128;

/// Timeout for idle segmented reassembly sessions.
const SEG_RECEIVER_TIMEOUT: Duration = Duration::from_secs(4);

/// Maximum negative SegmentAck retries during segmented response send.
const MAX_NEG_SEGMENT_ACK_RETRIES: u8 = 3;

/// Default timeout while waiting for SegmentACK during segmented response send.
const DEFAULT_APDU_SEGMENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default retransmission budget for segmented response segments.
const DEFAULT_APDU_SEGMENT_RETRIES: u8 = MAX_NEG_SEGMENT_ACK_RETRIES;

/// Default number of APDU retries for confirmed COV notifications.
const DEFAULT_APDU_RETRIES: u8 = 3;

type TsmPeer = (MacAddr, Option<NpduAddress>);
type TsmKey = (MacAddr, Option<NpduAddress>, u8);

// ---------------------------------------------------------------------------
// Server-side Transaction State Machine (TSM) for outgoing confirmed requests
// ---------------------------------------------------------------------------

include!("definitions.rs");
include!("runtime.rs");
