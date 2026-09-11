use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::{
    DeviceKey, DevicePath, OperationId, OutcomeError, PropertyRead, RuntimeError, WorkPriority,
};

/// Policy controlling whether a runtime cache may satisfy a read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshnessPolicy {
    /// Always issue BACnet wire I/O.
    WireOnly,
    /// Accept an observed value no older than this duration.
    MaxAge(Duration),
}

/// Provenance for one returned read value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadSource {
    /// Value came from this operation's BACnet wire exchange.
    Wire,
    /// Value came from the runtime observation cache.
    Cache,
}

/// One property in an ordered native read batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadBatchItem {
    /// Stable caller position.
    pub input_index: usize,
    /// Exact attachment-qualified device.
    pub device: DeviceKey,
    /// Property coordinates and planning category.
    pub read: PropertyRead,
    /// Cache acceptance policy.
    pub freshness: FreshnessPolicy,
}

/// Coarse read-batch request.
#[derive(Clone, Debug)]
pub struct ReadBatch {
    /// Ordered inputs, possibly spanning devices and attachments.
    pub items: Vec<ReadBatchItem>,
    /// Absolute operation deadline.
    pub deadline: Instant,
    /// Runtime scheduling lane.
    pub priority: WorkPriority,
}

/// One ordered read result with path and cache/wire provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOutcome {
    /// Original input position.
    pub input_index: usize,
    /// Exact device used.
    pub device: DeviceKey,
    /// Exact selected path, when device resolution succeeded.
    pub path: Option<DevicePath>,
    /// Raw application-tagged BACnet value.
    pub raw_value: Option<Vec<u8>>,
    /// Wire or cache provenance for a successful value.
    pub source: Option<ReadSource>,
    /// Structured per-item failure.
    pub error: Option<OutcomeError>,
}

/// One authorized property write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteBatchItem {
    /// Stable caller position.
    pub input_index: usize,
    /// Exact attachment-qualified device.
    pub device: DeviceKey,
    /// Property coordinates; `value_category` is ignored for writes.
    pub target: PropertyRead,
    /// Raw application-tagged value; application Null relinquishes a priority.
    pub raw_value: Vec<u8>,
    /// Optional BACnet command priority, 1 through 16.
    pub bacnet_priority: Option<u8>,
    /// Python authorization decision identifier retained for audit correlation.
    pub authorization_id: String,
    /// Read the property back and compare exact raw bytes after a successful write.
    pub verify_readback: bool,
}

/// Coarse ordered write-batch request.
#[derive(Clone, Debug)]
pub struct WriteBatch {
    /// Pre-authorized writes.
    pub items: Vec<WriteBatchItem>,
    /// Absolute operation deadline.
    pub deadline: Instant,
    /// Runtime scheduling lane.
    pub priority: WorkPriority,
}

/// One ordered write result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteOutcome {
    /// Original input position.
    pub input_index: usize,
    /// Exact device used.
    pub device: DeviceKey,
    /// Exact selected path, when device resolution succeeded.
    pub path: Option<DevicePath>,
    /// Authorization correlation value copied from the request.
    pub authorization_id: String,
    /// Whether the BACnet write completed successfully.
    pub written: bool,
    /// Raw readback when requested and successful.
    pub readback: Option<Vec<u8>>,
    /// Structured per-item failure.
    pub error: Option<OutcomeError>,
}

/// Submitted read operation whose ID is available before awaiting completion.
#[derive(Debug)]
pub struct ReadBatchOperation {
    /// ID accepted by `BacnetRuntime::cancel`.
    pub id: OperationId,
    receiver: oneshot::Receiver<Result<Vec<ReadOutcome>, RuntimeError>>,
}

impl ReadBatchOperation {
    pub(crate) fn new(
        id: OperationId,
        receiver: oneshot::Receiver<Result<Vec<ReadOutcome>, RuntimeError>>,
    ) -> Self {
        Self { id, receiver }
    }

    /// Awaits the final ordered batch result.
    pub async fn result(self) -> Result<Vec<ReadOutcome>, RuntimeError> {
        self.receiver
            .await
            .unwrap_or_else(|_| Err(RuntimeError::internal("read operation task ended")))
    }
}

/// Submitted write operation whose ID is available before awaiting completion.
#[derive(Debug)]
pub struct WriteBatchOperation {
    /// ID accepted by `BacnetRuntime::cancel`.
    pub id: OperationId,
    receiver: oneshot::Receiver<Result<Vec<WriteOutcome>, RuntimeError>>,
}

impl WriteBatchOperation {
    pub(crate) fn new(
        id: OperationId,
        receiver: oneshot::Receiver<Result<Vec<WriteOutcome>, RuntimeError>>,
    ) -> Self {
        Self { id, receiver }
    }

    /// Awaits the final ordered batch result.
    pub async fn result(self) -> Result<Vec<WriteOutcome>, RuntimeError> {
        self.receiver
            .await
            .unwrap_or_else(|_| Err(RuntimeError::internal("write operation task ended")))
    }
}

pub(crate) fn outcome_error(error: &RuntimeError) -> OutcomeError {
    OutcomeError {
        code: error.code,
        retryable: error.retryable,
        bacnet_class: None,
        bacnet_code: None,
        message: error.message.clone(),
    }
}
