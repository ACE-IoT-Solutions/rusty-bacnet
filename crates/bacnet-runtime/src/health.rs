use crate::AttachmentId;

/// Runtime view of managed B/IP foreign-device registration.
///
/// Upstream 0.11 performs registration in the transport background but does
/// not currently expose its live state. These variants preserve the runtime
/// API for the point at which that transport telemetry is available again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignDeviceRegistrationState {
    /// The initial registration is awaiting a result.
    Pending,
    /// The BBMD accepted registration.
    Registered,
    /// The BBMD rejected registration.
    Rejected,
    /// No successful registration is within its TTL.
    Expired,
}

/// Point-in-time foreign-device registration status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignDeviceRegistrationStatus {
    /// Registration state.
    pub state: ForeignDeviceRegistrationState,
    /// Most recent BVLC result, if available.
    pub last_result_code: Option<bacnet_types::enums::BvlcResultCode>,
    /// Whole seconds until the next renewal, if available.
    pub seconds_to_renewal: Option<u64>,
}

/// Lifecycle state of one attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentState {
    /// Configuration is registered but no transport is running yet.
    Configured,
    /// The transport is starting.
    Starting,
    /// The attachment is healthy and accepts work.
    Running,
    /// The attachment is quiescing.
    Stopping,
    /// The attachment is stopped.
    Stopped,
    /// The attachment failed while other attachments remain isolated.
    Failed,
}

/// Health summary for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentHealth {
    /// Durable attachment identity.
    pub id: AttachmentId,
    /// Operator label.
    pub label: String,
    /// Current lifecycle state.
    pub state: AttachmentState,
    /// Most recent stable error category, including live registration degradation.
    ///
    /// Rejected or expired foreign-device registration reports
    /// `ForeignDeviceRegistrationFailed` while the attachment remains `Running`;
    /// the error clears after a successful renewal.
    pub last_error: Option<crate::ErrorCode>,
    /// Foreign-device registration status for configured B/IP foreign attachments.
    ///
    /// This remains `None` on upstream 0.11 because the transport does not
    /// expose its background registration state.
    pub foreign_device_registration: Option<ForeignDeviceRegistrationStatus>,
}

/// Runtime health snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeHealth {
    /// Runtime generation. Reconcile will advance this value.
    pub generation: u64,
    /// Whether new commands are accepted.
    pub accepting_commands: bool,
    /// Total live runtime, client, and transport background tasks.
    pub task_count: usize,
    /// Live tasks owned by the runtime supervisor.
    pub supervisor_task_count: usize,
    /// Live client dispatch and transport I/O/reconnect tasks.
    pub attachment_task_count: usize,
    /// Number of retained events awaiting collection.
    pub event_queue_depth: usize,
    /// Total events evicted from the bounded queue.
    pub event_lag_count: u64,
    /// Total COV notifications skipped by lagging attachment receivers.
    pub cov_notification_lag_count: u64,
    /// Total I-Am observations skipped by lagging attachment receivers.
    pub i_am_observation_lag_count: u64,
    /// Distinct BACnet device instances in the cross-attachment index.
    pub device_count: usize,
    /// Attachment-qualified observations, including duplicates.
    pub device_observation_count: usize,
    /// Devices with learned scan capability state.
    pub capability_count: usize,
    /// Fresh raw property values retained for explicit freshness policies.
    pub cached_value_count: usize,
    /// Desired managed COV subscriptions in the current observation revision.
    pub observation_count: usize,
    /// Attachment health in deterministic configuration order.
    pub attachments: Vec<AttachmentHealth>,
}

/// Deterministic shutdown result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopReport {
    /// True when this call performed the transition to stopped.
    pub newly_stopped: bool,
    /// Tasks that exited within the cooperative grace period.
    pub tasks_joined: usize,
    /// Tasks aborted after the grace period.
    pub tasks_aborted: usize,
    /// Attachments remaining after shutdown; must be zero.
    pub remaining_attachments: usize,
}

/// Result of one revisioned attachment reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileReport {
    /// Applied caller revision.
    pub revision: u64,
    /// Runtime generation after the operation.
    pub generation: u64,
    /// Newly created attachments.
    pub added: Vec<AttachmentId>,
    /// Reconfigured or relabeled attachments.
    pub updated: Vec<AttachmentId>,
    /// Removed attachments.
    pub removed: Vec<AttachmentId>,
    /// True when an identical revision/configuration was replayed.
    pub idempotent: bool,
}
