use std::time::Duration;

use crate::{DeviceKey, PlanLimits, PropertyCatalog, ScanSnapshot};

/// One coarse request that enumerates a Device object and expands its catalog in Rust.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceScanRequest {
    /// Exact attachment-qualified device.
    pub device: DeviceKey,
    /// Immutable reviewed edge property catalog.
    pub catalog: PropertyCatalog,
    /// Maximum accepted object-list cardinality.
    pub max_objects: usize,
    /// Native RPM planning limits for enriched properties.
    pub limits: PlanLimits,
    /// Minimum interval between scan progress events.
    pub progress_interval: Duration,
}

/// Complete native device scan result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceScanSnapshot {
    /// Ordered object identifiers from the Device object-list.
    pub objects: Vec<(u16, u32)>,
    /// Enriched property scan in catalog order.
    pub properties: ScanSnapshot,
}
