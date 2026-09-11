//! Supervised, multi-attachment BACnet runtime.

mod batch;
mod cache;
mod capability;
mod catalog;
mod config;
mod device;
mod device_scan;
mod discovery;
mod error;
mod event;
mod executor;
mod health;
mod observation;
mod planner;
mod registry;
mod runtime;
mod scan;
mod scheduler;
mod supervisor;
mod topology;
mod transport;

pub use batch::{
    FreshnessPolicy, ReadBatch, ReadBatchItem, ReadBatchOperation, ReadOutcome, ReadSource,
    WriteBatch, WriteBatchItem, WriteBatchOperation, WriteOutcome,
};
pub use capability::{
    CapabilityCache, DeviceCapabilities, ObjectListAttempt, ObjectListStrategy, Support,
};
pub use catalog::{ObjectCatalog, PropertyCatalog, PropertySpec, ScanPriority};
pub use config::{
    AttachmentConfig, AttachmentId, BipConfig, MstpConfig, RuntimeConfig, ScConfig, TransportConfig,
};
pub use device::{
    DeviceIndex, DeviceIndexChange, DeviceIndexChangeKind, DeviceKey, DeviceObservation,
    DevicePath, DeviceRestoreReport, DeviceSelection, PersistedDevice,
};
pub use device_scan::{DeviceScanRequest, DeviceScanSnapshot};
pub use discovery::{DiscoveryRequest, DiscoverySnapshot, RouterObservation};
pub use error::{ErrorCode, RuntimeError};
pub use event::{
    EventBatch, EventKind, IAmObservation, ObservedCovValue, RuntimeEvent,
    UnsolicitedCovNotification,
};
pub use executor::{OutcomeError, PropertyOutcome, ScanExecutor};
pub use health::{
    AttachmentHealth, AttachmentState, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus, ReconcileReport, RuntimeHealth, StopReport,
};
pub use observation::{
    CovValue, ObservationKey, ObservationPlan, ObservationReport, ObservationSpec,
};
pub use planner::{PlanLimits, PlannedProperty, PropertyRead, RpmBatch, ScanPlanner};
pub use runtime::BacnetRuntime;
pub use scan::{ScanRequest, ScanSnapshot};
pub use scheduler::{CancelReport, Dispatch, OperationId, Scheduled, WorkPriority, WorkScheduler};
pub use topology::{
    AttachmentTopology, BbmdSnapshot, BbmdTarget, BdtRecord, FdtRecord, RuntimeTopology,
    TopologyRequest,
};
pub use transport::RuntimeTransport;
