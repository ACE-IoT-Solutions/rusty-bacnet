use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bacnet_encoding::primitives::encode_property_value;
use bacnet_runtime::{
    AttachmentConfig, AttachmentId, BacnetRuntime, BbmdTarget, BipConfig, DeviceKey, DevicePath,
    DeviceScanRequest, DiscoveryRequest, FreshnessPolicy, MstpConfig, ObservationKey,
    ObservationPlan, ObservationSpec, OperationId, PersistedDevice, PlanLimits, PropertyCatalog,
    PropertyRead, ReadBatch, ReadBatchItem, ReadBatchOperation, RuntimeConfig, RuntimeError,
    ScConfig, ScanRequest, TopologyRequest, TransportConfig, WorkPriority, WriteBatch,
    WriteBatchItem, WriteBatchOperation,
};
use bytes::BytesMut;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyType;

use crate::types::{PyForeignDeviceStatus, PyObjectIdentifier, PyPropertyValue};

mod attachments;
mod conversions;
mod core;
mod helpers;
mod io;
mod lifecycle;
mod models;
mod operations;

pub use attachments::{
    PyRuntimeAttachment, PyRuntimeMstpAttachment, PyRuntimePersistedDevice,
    PyRuntimeReconcileReport, PyRuntimeScAttachment,
};
pub use core::PyBACnetRuntime;
pub use io::{
    PyRuntimeBatchOutcome, PyRuntimeCancelReport, PyRuntimeObservation, PyRuntimeObservationReport,
    PyRuntimeRead, PyRuntimeReadOperation, PyRuntimeWrite, PyRuntimeWriteOperation,
};
pub use models::{
    PyRuntimeAttachmentTopology, PyRuntimeBbmdSnapshot, PyRuntimeBdtEntry, PyRuntimeCovValue,
    PyRuntimeCrossingCounters, PyRuntimeDeviceRestoreReport, PyRuntimeDeviceScanSnapshot,
    PyRuntimeDiscoveredDevice, PyRuntimeDiscoveredRouter, PyRuntimeDiscoverySnapshot,
    PyRuntimeEvent, PyRuntimeEventBatch, PyRuntimeFdtEntry, PyRuntimeHealth,
    PyRuntimeScannedObject, PyRuntimeTopologySnapshot, PyScanOutcome, PyScanSnapshot,
};

use attachments::PyRuntimeAttachmentConfig;
use conversions::*;
use helpers::*;
use io::{CrossingCounters, ScanRead};
