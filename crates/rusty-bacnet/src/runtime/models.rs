use super::*;

/// Result of one native persisted-device restore operation.
#[pyclass(name = "RuntimeDeviceRestoreReport", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeDeviceRestoreReport {
    #[pyo3(get)]
    pub(super) added: usize,
    #[pyo3(get)]
    pub(super) updated: usize,
    #[pyo3(get)]
    pub(super) unchanged: usize,
    #[pyo3(get)]
    pub(super) index_revision: u64,
}

/// Point-in-time runtime health returned without BACnet I/O.
#[pyclass(name = "RuntimeHealth", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeHealth {
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) accepting_commands: bool,
    #[pyo3(get)]
    pub(super) task_count: usize,
    #[pyo3(get)]
    pub(super) supervisor_task_count: usize,
    #[pyo3(get)]
    pub(super) attachment_task_count: usize,
    #[pyo3(get)]
    pub(super) event_queue_depth: usize,
    #[pyo3(get)]
    pub(super) event_lag_count: u64,
    #[pyo3(get)]
    pub(super) cov_notification_lag_count: u64,
    #[pyo3(get)]
    pub(super) i_am_observation_lag_count: u64,
    #[pyo3(get)]
    pub(super) device_count: usize,
    #[pyo3(get)]
    pub(super) device_observation_count: usize,
    #[pyo3(get)]
    pub(super) capability_count: usize,
    #[pyo3(get)]
    pub(super) cached_value_count: usize,
    #[pyo3(get)]
    pub(super) observation_count: usize,
    #[pyo3(get)]
    pub(super) attachment_ids: Vec<u128>,
    #[pyo3(get)]
    pub(super) attachment_states: Vec<String>,
    #[pyo3(get)]
    pub(super) attachment_error_codes: Vec<Option<String>>,
    #[pyo3(get)]
    pub(super) foreign_device_statuses: Vec<Option<PyForeignDeviceStatus>>,
}

/// Counts Python/Rust crossings and batch cardinality.
#[pyclass(name = "RuntimeCrossingCounters", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeCrossingCounters {
    #[pyo3(get)]
    pub(super) calls: u64,
    #[pyo3(get)]
    pub(super) input_items: u64,
    #[pyo3(get)]
    pub(super) output_items: u64,
}

/// One attachment-qualified device observation with lossless path provenance.
#[pyclass(name = "RuntimeDiscoveredDevice", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeDiscoveredDevice {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) device_instance: u32,
    #[pyo3(get)]
    pub(super) selected: bool,
    #[pyo3(get)]
    pub(super) path_kind: String,
    #[pyo3(get)]
    pub(super) path_mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) routed_dnet: Option<u16>,
    #[pyo3(get)]
    pub(super) routed_dadr: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) vendor_id: u16,
    #[pyo3(get)]
    pub(super) max_apdu_length: u16,
    #[pyo3(get)]
    pub(super) revision: u64,
}

/// One router announcement collected during runtime discovery.
#[pyclass(name = "RuntimeDiscoveredRouter", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeDiscoveredRouter {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) path_kind: String,
    #[pyo3(get)]
    pub(super) path_mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) routed_dnet: Option<u16>,
    #[pyo3(get)]
    pub(super) routed_dadr: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) networks: Vec<u16>,
}

/// Coarse discovery snapshot retaining alternates, routers, and partial errors.
#[pyclass(name = "RuntimeDiscoverySnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeDiscoverySnapshot {
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) index_revision: u64,
    #[pyo3(get)]
    pub(super) devices: Vec<PyRuntimeDiscoveredDevice>,
    #[pyo3(get)]
    pub(super) routers: Vec<PyRuntimeDiscoveredRouter>,
    #[pyo3(get)]
    pub(super) error_codes: Vec<String>,
}

/// One BDT row from a native topology walk.
#[pyclass(name = "RuntimeBdtEntry", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeBdtEntry {
    #[pyo3(get)]
    pub(super) ip: Vec<u8>,
    #[pyo3(get)]
    pub(super) port: u16,
    #[pyo3(get)]
    pub(super) broadcast_mask: Vec<u8>,
}

/// One FDT row from a native topology walk.
#[pyclass(name = "RuntimeFdtEntry", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeFdtEntry {
    #[pyo3(get)]
    pub(super) ip: Vec<u8>,
    #[pyo3(get)]
    pub(super) port: u16,
    #[pyo3(get)]
    pub(super) ttl: u16,
    #[pyo3(get)]
    pub(super) seconds_remaining: u16,
}

/// Tables read from one reachable BBMD.
#[pyclass(name = "RuntimeBbmdSnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeBbmdSnapshot {
    #[pyo3(get)]
    pub(super) mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) bdt: Vec<PyRuntimeBdtEntry>,
    #[pyo3(get)]
    pub(super) fdt: Vec<PyRuntimeFdtEntry>,
}

/// Topology owned by one runtime attachment.
#[pyclass(name = "RuntimeAttachmentTopology", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeAttachmentTopology {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) local_mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) bbmds: Vec<PyRuntimeBbmdSnapshot>,
    #[pyo3(get)]
    pub(super) routers: Vec<PyRuntimeDiscoveredRouter>,
    #[pyo3(get)]
    pub(super) truncated: bool,
}

/// Coarse native topology result with partial error codes.
#[pyclass(name = "RuntimeTopologySnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeTopologySnapshot {
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) attachments: Vec<PyRuntimeAttachmentTopology>,
    #[pyo3(get)]
    pub(super) error_codes: Vec<String>,
}

/// One raw value carried by a runtime COV event.
#[pyclass(name = "RuntimeCovValue", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeCovValue {
    #[pyo3(get)]
    pub(super) property_id: u32,
    #[pyo3(get)]
    pub(super) array_index: Option<u32>,
    #[pyo3(get)]
    pub(super) value: Option<PyPropertyValue>,
    #[pyo3(get)]
    pub(super) raw_value: Vec<u8>,
    #[pyo3(get)]
    pub(super) priority: Option<u8>,
}

/// One typed event from the bounded runtime stream.
#[pyclass(name = "RuntimeEvent", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeEvent {
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) sequence: u64,
    #[pyo3(get)]
    pub(super) attachment_id: Option<u128>,
    #[pyo3(get)]
    pub(super) kind: String,
    #[pyo3(get)]
    pub(super) device_instance: Option<u32>,
    #[pyo3(get)]
    pub(super) completed: Option<usize>,
    #[pyo3(get)]
    pub(super) total: Option<usize>,
    #[pyo3(get)]
    pub(super) final_update: Option<bool>,
    #[pyo3(get)]
    pub(super) lost_count: Option<u64>,
    #[pyo3(get)]
    pub(super) error_code: Option<String>,
    #[pyo3(get)]
    pub(super) observation_process_id: Option<u32>,
    #[pyo3(get)]
    pub(super) subscriber_process_identifier: Option<u32>,
    #[pyo3(get)]
    pub(super) initiating_device_identifier: Option<PyObjectIdentifier>,
    #[pyo3(get)]
    pub(super) monitored_object_identifier: Option<PyObjectIdentifier>,
    #[pyo3(get)]
    pub(super) time_remaining: Option<u32>,
    #[pyo3(get)]
    pub(super) delivery: Option<String>,
    #[pyo3(get)]
    pub(super) object_identifier: Option<u32>,
    #[pyo3(get)]
    pub(super) max_apdu_length: Option<u32>,
    #[pyo3(get)]
    pub(super) segmentation_supported: Option<u8>,
    #[pyo3(get)]
    pub(super) vendor_id: Option<u16>,
    #[pyo3(get)]
    pub(super) udp_source_ip: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) udp_source_port: Option<u16>,
    #[pyo3(get)]
    pub(super) source_mac: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) source_network: Option<u16>,
    #[pyo3(get)]
    pub(super) source_address: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) bvlc_function: Option<u8>,
    #[pyo3(get)]
    pub(super) forwarded_from_ip: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) forwarded_from_port: Option<u16>,
    #[pyo3(get)]
    pub(super) timestamp: Option<f64>,
    #[pyo3(get)]
    pub(super) values: Vec<PyRuntimeCovValue>,
}

/// One bounded retrieval from the runtime event stream.
#[pyclass(name = "RuntimeEventBatch", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeEventBatch {
    #[pyo3(get)]
    pub(super) events: Vec<PyRuntimeEvent>,
}

/// One Python-visible outcome produced by the native scan engine.
#[pyclass(name = "ScanOutcome", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyScanOutcome {
    #[pyo3(get)]
    pub(super) input_index: usize,
    #[pyo3(get)]
    pub(super) object_type: u16,
    #[pyo3(get)]
    pub(super) object_instance: u32,
    #[pyo3(get)]
    pub(super) property_id: u32,
    #[pyo3(get)]
    pub(super) array_index: Option<u32>,
    #[pyo3(get)]
    pub(super) value_category: String,
    #[pyo3(get)]
    pub(super) value: Option<PyPropertyValue>,
    #[pyo3(get)]
    pub(super) raw_value: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) error_code: Option<String>,
    #[pyo3(get)]
    pub(super) retryable: bool,
    #[pyo3(get)]
    pub(super) bacnet_class: Option<u32>,
    #[pyo3(get)]
    pub(super) bacnet_code: Option<u32>,
}

/// Final Python-visible result of one coarse native scan call.
#[pyclass(name = "ScanSnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyScanSnapshot {
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) device_instance: u32,
    #[pyo3(get)]
    pub(super) path_kind: String,
    #[pyo3(get)]
    pub(super) path_mac: Vec<u8>,
    #[pyo3(get)]
    pub(super) routed_dnet: Option<u16>,
    #[pyo3(get)]
    pub(super) routed_dadr: Option<Vec<u8>>,
    #[pyo3(get)]
    pub(super) outcomes: Vec<PyScanOutcome>,
    #[pyo3(get)]
    pub(super) rpm_attempts: usize,
    #[pyo3(get)]
    pub(super) rp_fallbacks: usize,
    #[pyo3(get)]
    pub(super) elapsed_ms: u64,
}

/// One object identifier enumerated by a native device scan.
#[pyclass(name = "RuntimeScannedObject", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeScannedObject {
    #[pyo3(get)]
    pub(super) object_type: u16,
    #[pyo3(get)]
    pub(super) object_instance: u32,
}

/// Complete native object-list and catalog-enrichment result.
#[pyclass(name = "RuntimeDeviceScanSnapshot", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeDeviceScanSnapshot {
    #[pyo3(get)]
    pub(super) objects: Vec<PyRuntimeScannedObject>,
    #[pyo3(get)]
    pub(super) properties: PyScanSnapshot,
}
