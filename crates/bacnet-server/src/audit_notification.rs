//! Receiver policy for confirmed and unconfirmed AuditNotification services.

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// Local maximum accepted AuditNotification service payload.
pub const MAX_AUDIT_NOTIFICATION_BYTES: usize = 64 * 1024;
/// Local maximum number of notifications in one accepted request.
pub const MAX_AUDIT_NOTIFICATIONS: usize = 256;

/// Transport provenance and decoded content supplied to
/// [`ServerPolicy::authorize_audit_notification`](crate::ServerPolicy::authorize_audit_notification).
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance for this ingress; the source of
    /// the principal, if any.
    pub provenance: TransportProvenance,
    /// Outer Confirmed-Request invoke identifier.
    pub invoke_id: u8,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

impl AuditNotificationAuthorizationContext {
    /// The verified direct-SC leaf certificate's principal; `None` for every
    /// other ingress, hub-relayed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }
}

/// Transport provenance and decoded content supplied to
/// [`ServerPolicy::authorize_unconfirmed_audit_notification`](crate::ServerPolicy::authorize_unconfirmed_audit_notification).
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
#[derive(Debug, Clone, PartialEq)]
pub struct UnconfirmedAuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance for this ingress; the source of
    /// the principal, if any.
    pub provenance: TransportProvenance,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

impl UnconfirmedAuditNotificationAuthorizationContext {
    /// The verified direct-SC leaf certificate's principal; `None` for every
    /// other ingress, hub-relayed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }
}
