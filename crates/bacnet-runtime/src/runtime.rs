use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::primitives::PropertyValue;
use futures_util::future::join_all;
use tokio::sync::{oneshot, watch, Mutex, Notify, RwLock, Semaphore};

use crate::cache::ValueCache;
use crate::config::validate_attachments;
use crate::discovery::{device_observation, router_observation};
use crate::event::EventBus;
use crate::observation::ObservationRegistry;
use crate::registry::AttachmentRegistry;
use crate::supervisor::Supervisor;
use crate::{
    AttachmentConfig, AttachmentTopology, CancelReport, CapabilityCache, CovValue, DeviceIndex,
    DeviceScanRequest, DeviceScanSnapshot, DiscoveryRequest, DiscoverySnapshot, Dispatch,
    EventBatch, EventKind, FreshnessPolicy, ObjectListAttempt, ObjectListStrategy, ObservationKey,
    ObservationPlan, ObservationReport, ObservationSpec, OperationId, ReadBatch,
    ReadBatchOperation, ReadOutcome, ReadSource, ReconcileReport, RuntimeConfig, RuntimeError,
    RuntimeHealth, RuntimeTopology, ScanExecutor, ScanPlanner, ScanRequest, ScanSnapshot,
    StopReport, Support, TopologyRequest, WorkScheduler, WriteBatch, WriteBatchItem,
    WriteBatchOperation, WriteOutcome,
};

enum CovRollback {
    Subscribe(ObservationSpec),
    Unsubscribe(ObservationSpec),
}

#[derive(Debug)]
struct CovPumpControl {
    cancel: watch::Sender<bool>,
    attachment_epoch: u64,
    pump_token: u64,
}

#[derive(Debug)]
enum RuntimeCommand {
    Read {
        request: ReadBatch,
        response: Option<oneshot::Sender<Result<Vec<ReadOutcome>, RuntimeError>>>,
    },
    Write {
        request: WriteBatch,
        response: Option<oneshot::Sender<Result<Vec<WriteOutcome>, RuntimeError>>>,
    },
}

impl RuntimeCommand {
    async fn execute(&mut self, runtime: &BacnetRuntime) {
        match self {
            Self::Read { request, response } => {
                let result = runtime.execute_read_batch(request.clone()).await;
                if let Some(response) = response.take() {
                    let _ = response.send(result);
                }
            }
            Self::Write { request, response } => {
                let result = runtime.execute_write_batch(request.clone()).await;
                if let Some(response) = response.take() {
                    let _ = response.send(result);
                }
            }
        }
    }

    fn fail(mut self, error: RuntimeError) {
        match &mut self {
            Self::Read { response, .. } => {
                if let Some(response) = response.take() {
                    let _ = response.send(Err(error));
                }
            }
            Self::Write { response, .. } => {
                if let Some(response) = response.take() {
                    let _ = response.send(Err(error));
                }
            }
        }
    }
}

#[derive(Debug)]
struct RuntimeInner {
    generation: AtomicU64,
    config_revision: AtomicU64,
    stopped: AtomicBool,
    shutdown_timeout: Duration,
    registry: RwLock<AttachmentRegistry>,
    device_index: RwLock<DeviceIndex>,
    capabilities: RwLock<CapabilityCache>,
    values: RwLock<ValueCache>,
    observations: RwLock<ObservationRegistry>,
    pending_observations: RwLock<std::collections::BTreeMap<ObservationKey, ObservationSpec>>,
    cov_renewals: Mutex<std::collections::BTreeMap<ObservationKey, watch::Sender<bool>>>,
    cov_pumps: Mutex<std::collections::BTreeMap<crate::AttachmentId, CovPumpControl>>,
    cov_pump_lifecycle: Mutex<()>,
    next_cov_pump_token: AtomicU64,
    i_am_pumps: Mutex<std::collections::BTreeMap<crate::AttachmentId, IAmPumpControl>>,
    cov_last: Mutex<std::collections::BTreeMap<ObservationKey, Vec<CovValue>>>,
    cov_notification_lag_count: AtomicU64,
    i_am_observation_lag_count: AtomicU64,
    work_queue: Mutex<WorkScheduler<RuntimeCommand>>,
    work_notify: Notify,
    work_slots: Arc<Semaphore>,
    inflight: Mutex<std::collections::BTreeMap<OperationId, watch::Sender<bool>>>,
    events: EventBus,
    supervisor: Supervisor,
    stop_lock: Mutex<()>,
    reconcile_lock: Mutex<()>,
    observation_lock: Mutex<()>,
}

#[derive(Debug)]
struct IAmPumpControl {
    cancel: watch::Sender<bool>,
    attachment_epoch: u64,
}

/// Supervised multi-attachment BACnet runtime.
#[derive(Clone, Debug)]
pub struct BacnetRuntime {
    inner: Arc<RuntimeInner>,
}

mod batch;
mod helpers;
mod lifecycle;
mod observation;
mod scan;

use helpers::*;

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
