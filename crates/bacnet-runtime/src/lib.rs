//! A bounded, supervised BACnet/IP runtime.
//!
//! This first upstream-0.13 slice deliberately owns only BACnet/IP. MS/TP,
//! BACnet/SC, router discovery, raw I-Am observation, COV supervision and
//! topology management are rejected or deferred rather than represented as
//! partially working runtime features.

mod attachment;
mod batch;
mod config;
mod error;
mod health;
mod runtime;

pub use batch::{
    DevicePath, OperationId, PropertyTarget, ReadBatch, ReadBatchItem, ReadBatchOperation,
    ReadOutcome, WriteBatch, WriteBatchItem, WriteBatchOperation, WriteOutcome,
};
pub use config::{
    AttachmentConfig, AttachmentId, BipConfig, MstpConfig, RuntimeConfig, ScConfig, TransportConfig,
};
pub use error::{ErrorCode, RuntimeError};
pub use health::{
    AttachmentHealth, AttachmentState, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus, ReconcileReport, RuntimeHealth, StopReport,
};
pub use runtime::BacnetRuntime;
