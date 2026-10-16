use std::time::Duration;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::task::JoinHandle;

use crate::{AttachmentId, RuntimeError};

/// Runtime operation identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(pub(crate) u64);

/// Direct B/IP destination and ReadProperty addressing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyTarget {
    /// Six-octet B/IP MAC (IPv4 address followed by UDP port).
    pub destination_mac: Vec<u8>,
    /// Object identifier.
    pub object_identifier: ObjectIdentifier,
    /// Property identifier.
    pub property_identifier: PropertyIdentifier,
    /// Optional array index.
    pub property_array_index: Option<u32>,
}

impl PropertyTarget {
    /// Creates a direct B/IP property target.
    pub fn new(
        destination_mac: impl Into<Vec<u8>>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Self {
        Self {
            destination_mac: destination_mac.into(),
            object_identifier,
            property_identifier,
            property_array_index,
        }
    }
}

/// One ordered read input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadBatchItem {
    /// Original caller position, preserved in the outcome.
    pub input_index: usize,
    /// Attachment to use.
    pub attachment_id: AttachmentId,
    /// Direct B/IP property target.
    pub target: PropertyTarget,
}

/// An ordered bounded read batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadBatch {
    /// Inputs in caller order.
    pub items: Vec<ReadBatchItem>,
    /// Maximum time allowed for the whole batch.
    pub timeout: Duration,
}

impl ReadBatch {
    /// Creates a read batch with a ten-second deadline.
    pub fn new(items: Vec<ReadBatchItem>) -> Self {
        Self {
            items,
            timeout: Duration::from_secs(10),
        }
    }
}

/// One ordered read outcome. A failed item does not discard later outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOutcome {
    /// Original caller position.
    pub input_index: usize,
    /// Attachment used by the input.
    pub attachment_id: AttachmentId,
    /// Raw application-tagged property value on success.
    pub raw_value: Option<Vec<u8>>,
    /// Typed error on item failure.
    pub error: Option<RuntimeError>,
}

/// One ordered write input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteBatchItem {
    /// Original caller position, preserved in the outcome.
    pub input_index: usize,
    /// Attachment to use.
    pub attachment_id: AttachmentId,
    /// Direct B/IP property target.
    pub target: PropertyTarget,
    /// Raw application-tagged value.
    pub raw_value: Vec<u8>,
    /// Optional BACnet write priority.
    pub priority: Option<u8>,
}

/// An ordered bounded write batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteBatch {
    /// Inputs in caller order.
    pub items: Vec<WriteBatchItem>,
    /// Maximum time allowed for the whole batch.
    pub timeout: Duration,
}

impl WriteBatch {
    /// Creates a write batch with a ten-second deadline.
    pub fn new(items: Vec<WriteBatchItem>) -> Self {
        Self {
            items,
            timeout: Duration::from_secs(10),
        }
    }
}

/// One ordered write outcome. A failed item does not discard later outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteOutcome {
    /// Original caller position.
    pub input_index: usize,
    /// Attachment used by the input.
    pub attachment_id: AttachmentId,
    /// Whether BACnet acknowledged the write.
    pub written: bool,
    /// Typed error on item failure.
    pub error: Option<RuntimeError>,
}

/// Future-like handle for one scheduled read batch.
pub struct ReadBatchOperation {
    /// Operation identifier.
    pub id: OperationId,
    task: Option<JoinHandle<Result<Vec<ReadOutcome>, RuntimeError>>>,
}

impl ReadBatchOperation {
    pub(crate) fn new(
        id: OperationId,
        task: JoinHandle<Result<Vec<ReadOutcome>, RuntimeError>>,
    ) -> Self {
        Self {
            id,
            task: Some(task),
        }
    }

    /// Waits for the ordered batch result.
    pub async fn result(mut self) -> Result<Vec<ReadOutcome>, RuntimeError> {
        match self.task.take().expect("operation task present").await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Err(RuntimeError::stopped()),
            Err(error) => Err(RuntimeError::internal(error.to_string())),
        }
    }
}

impl Drop for ReadBatchOperation {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Future-like handle for one scheduled write batch.
pub struct WriteBatchOperation {
    /// Operation identifier.
    pub id: OperationId,
    task: Option<JoinHandle<Result<Vec<WriteOutcome>, RuntimeError>>>,
}

impl WriteBatchOperation {
    pub(crate) fn new(
        id: OperationId,
        task: JoinHandle<Result<Vec<WriteOutcome>, RuntimeError>>,
    ) -> Self {
        Self {
            id,
            task: Some(task),
        }
    }

    /// Waits for the ordered batch result.
    pub async fn result(mut self) -> Result<Vec<WriteOutcome>, RuntimeError> {
        match self.task.take().expect("operation task present").await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Err(RuntimeError::stopped()),
            Err(error) => Err(RuntimeError::internal(error.to_string())),
        }
    }
}

impl Drop for WriteBatchOperation {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
