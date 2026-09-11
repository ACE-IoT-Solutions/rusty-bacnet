use super::*;

/// Coarse supervised BACnet runtime. Python submits operations, not APDUs.
#[pyclass(name = "BACnetRuntime")]
pub struct PyBACnetRuntime {
    pub(super) inner: BacnetRuntime,
    pub(super) crossings: Arc<CrossingCounters>,
    pub(super) catalog: Arc<tokio::sync::RwLock<Option<PropertyCatalog>>>,
}
