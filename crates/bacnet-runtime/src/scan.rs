use std::time::Duration;

use crate::{DeviceKey, DevicePath, PlanLimits, PropertyOutcome, PropertyRead, RuntimeError};

/// One coarse native scan request for an attachment-qualified device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanRequest {
    /// Exact device observation to use; no cross-attachment ambiguity is allowed.
    pub device: DeviceKey,
    /// Ordered properties to execute inside Rust.
    pub reads: Vec<PropertyRead>,
    /// APDU and property-count bounds for native RPM planning.
    pub limits: PlanLimits,
    /// Minimum interval between non-final coarse progress events.
    pub progress_interval: Duration,
}

/// Final ordered result of one native scan operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanSnapshot {
    /// Runtime generation used for the operation.
    pub generation: u64,
    /// Exact device observation used.
    pub device: DeviceKey,
    /// Exact direct or routed path selected at operation start.
    pub path: DevicePath,
    /// Per-property values and errors in original input order.
    pub outcomes: Vec<PropertyOutcome>,
    /// Batch-level failures retained even when ReadProperty fallback succeeded.
    pub errors: Vec<RuntimeError>,
    /// Number of RPM batches attempted.
    pub rpm_attempts: usize,
    /// Number of individual ReadProperty fallbacks attempted.
    pub rp_fallbacks: usize,
    /// Total native operation duration.
    pub elapsed: Duration,
}
