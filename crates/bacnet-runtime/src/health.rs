use crate::AttachmentId;

/// Runtime view of managed B/IP foreign-device registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignDeviceRegistrationState {
    /// The first registration attempt has not completed.
    Pending,
    /// The transport holds a live foreign-device lease.
    Registered,
    /// The BBMD rejected the latest registration attempt.
    Rejected,
    /// No successful registration is currently within its TTL.
    Expired,
}

/// Point-in-time foreign-device registration status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignDeviceRegistrationStatus {
    /// Current lease state.
    pub state: ForeignDeviceRegistrationState,
    /// Most recent BVLC result, if available.
    pub last_result_code: Option<bacnet_bip_codec::BvlcResultCode>,
    /// Whole seconds until the next automatic attempt, if available.
    pub seconds_to_renewal: Option<u64>,
}

impl From<bacnet_transport::bip::ForeignRegistrationSnapshot> for ForeignDeviceRegistrationStatus {
    fn from(status: bacnet_transport::bip::ForeignRegistrationSnapshot) -> Self {
        use bacnet_transport::bip::ForeignRegistrationOutcome;

        let state = if status.registered {
            ForeignDeviceRegistrationState::Registered
        } else {
            match status.last_outcome {
                None | Some(ForeignRegistrationOutcome::Pending) => {
                    ForeignDeviceRegistrationState::Pending
                }
                Some(ForeignRegistrationOutcome::Refused) => {
                    ForeignDeviceRegistrationState::Rejected
                }
                Some(
                    ForeignRegistrationOutcome::Accepted
                    | ForeignRegistrationOutcome::TimedOut
                    | ForeignRegistrationOutcome::LocalError
                    | ForeignRegistrationOutcome::Cancelled,
                ) => ForeignDeviceRegistrationState::Expired,
            }
        };
        Self {
            state,
            last_result_code: status.last_result,
            seconds_to_renewal: status.next_attempt_in.map(|remaining| remaining.as_secs()),
        }
    }
}

/// Lifecycle state of one managed attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentState {
    /// Configuration is registered but the client is not running.
    Configured,
    /// The client is binding and starting.
    Starting,
    /// The client is available for work.
    Running,
    /// The client is being stopped.
    Stopping,
    /// The client is stopped.
    Stopped,
    /// The client failed while other attachments remain isolated.
    Failed,
}

/// Point-in-time health for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentHealth {
    /// Durable attachment identity.
    pub id: AttachmentId,
    /// Operator label.
    pub label: String,
    /// Lifecycle state.
    ///
    /// This is runtime lifecycle state, not a probe of the client's internal
    /// receive task. A foreign-device lease problem is reported separately in
    /// `last_error` while the attachment remains usable for unicast traffic.
    pub state: AttachmentState,
    /// Most recent error category.
    pub last_error: Option<crate::ErrorCode>,
    /// Local B/IP MAC (IPv4 address and UDP port) when known.
    pub local_mac: Option<Vec<u8>>,
    /// Registration state for configured B/IP foreign-device attachments.
    pub foreign_device_registration: Option<ForeignDeviceRegistrationStatus>,
}

/// Runtime-wide health snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeHealth {
    /// Runtime generation, incremented after successful reconciliation.
    pub generation: u64,
    /// Whether new work is accepted.
    pub accepting_commands: bool,
    /// Number of currently managed background tasks.
    pub task_count: usize,
    /// Number of runtime supervisor tasks. This slice has none.
    pub supervisor_task_count: usize,
    /// Number of attachment client/transport task groups.
    pub attachment_task_count: usize,
    /// Number of active scheduled batch operations.
    pub operation_count: usize,
    /// Event and discovery counters reserved for the broader runtime.
    pub event_queue_depth: usize,
    /// Reserved event lag count.
    pub event_lag_count: u64,
    /// Reserved COV lag count.
    pub cov_notification_lag_count: u64,
    /// Reserved I-Am observation lag count.
    pub i_am_observation_lag_count: u64,
    /// Reserved cross-attachment device count.
    pub device_count: usize,
    /// Reserved device observation count.
    pub device_observation_count: usize,
    /// Reserved capability count.
    pub capability_count: usize,
    /// Reserved cached-value count.
    pub cached_value_count: usize,
    /// Reserved observation count.
    pub observation_count: usize,
    /// Attachment health in deterministic ID order.
    pub attachments: Vec<AttachmentHealth>,
}

/// Result of stopping the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopReport {
    /// Whether this call performed the transition.
    pub newly_stopped: bool,
    /// Tasks stopped cooperatively.
    pub tasks_joined: usize,
    /// Tasks aborted after the shutdown deadline.
    pub tasks_aborted: usize,
    /// Attachments remaining after shutdown.
    pub remaining_attachments: usize,
}

/// Result of a complete attachment reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileReport {
    /// Applied caller revision.
    pub revision: u64,
    /// Runtime generation after reconciliation.
    pub generation: u64,
    /// Newly started attachments.
    pub added: Vec<AttachmentId>,
    /// Replaced or relabeled attachments.
    pub updated: Vec<AttachmentId>,
    /// Removed attachments.
    pub removed: Vec<AttachmentId>,
    /// True when desired configuration was identical.
    pub idempotent: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::bip::{ForeignRegistrationOutcome, ForeignRegistrationSnapshot};

    #[test]
    fn foreign_registration_reports_live_lease_before_last_attempt() {
        let mut snapshot = ForeignRegistrationSnapshot::default();
        snapshot.registered = true;
        snapshot.last_outcome = Some(ForeignRegistrationOutcome::Refused);
        let status = ForeignDeviceRegistrationStatus::from(snapshot);
        assert_eq!(status.state, ForeignDeviceRegistrationState::Registered);
    }

    #[test]
    fn foreign_registration_reports_refusal_without_live_lease() {
        let mut snapshot = ForeignRegistrationSnapshot::default();
        snapshot.last_outcome = Some(ForeignRegistrationOutcome::Refused);
        let status = ForeignDeviceRegistrationStatus::from(snapshot);
        assert_eq!(status.state, ForeignDeviceRegistrationState::Rejected);
    }
}
