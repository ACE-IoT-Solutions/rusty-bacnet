//! Python bindings for rusty-bacnet via PyO3.

use pyo3::prelude::*;

mod client;
mod errors;
mod hub;
mod mstp_py;
mod router;
mod runtime;
mod sc_identity;
mod server;
mod tls;
mod types;

/// The `rusty_bacnet` Python module.
#[pymodule]
fn rusty_bacnet(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register exception types
    errors::register(m)?;

    // Register type wrappers
    types::register(m)?;

    // Register client and server classes
    m.add_class::<client::BACnetClient>()?;
    m.add_class::<server::BACnetServer>()?;
    m.add_class::<router::PyBACnetRouter>()?;
    m.add_class::<router::PyRouterPortCounters>()?;
    m.add_class::<hub::PyScHub>()?;
    m.add_class::<runtime::PyBACnetRuntime>()?;
    m.add_class::<runtime::PyRuntimeAttachment>()?;
    m.add_class::<runtime::PyRuntimeMstpAttachment>()?;
    m.add_class::<runtime::PyRuntimeScAttachment>()?;
    m.add_class::<runtime::PyRuntimeReconcileReport>()?;
    m.add_class::<runtime::PyRuntimePersistedDevice>()?;
    m.add_class::<runtime::PyRuntimeDeviceRestoreReport>()?;
    m.add_class::<runtime::PyRuntimeHealth>()?;
    m.add_class::<runtime::PyRuntimeCrossingCounters>()?;
    m.add_class::<runtime::PyRuntimeDiscoveredDevice>()?;
    m.add_class::<runtime::PyRuntimeDiscoveredRouter>()?;
    m.add_class::<runtime::PyRuntimeDiscoverySnapshot>()?;
    m.add_class::<runtime::PyRuntimeBdtEntry>()?;
    m.add_class::<runtime::PyRuntimeFdtEntry>()?;
    m.add_class::<runtime::PyRuntimeBbmdSnapshot>()?;
    m.add_class::<runtime::PyRuntimeAttachmentTopology>()?;
    m.add_class::<runtime::PyRuntimeTopologySnapshot>()?;
    m.add_class::<runtime::PyRuntimeCovValue>()?;
    m.add_class::<runtime::PyRuntimeEvent>()?;
    m.add_class::<runtime::PyRuntimeEventBatch>()?;
    m.add_class::<runtime::PyRuntimeRead>()?;
    m.add_class::<runtime::PyRuntimeWrite>()?;
    m.add_class::<runtime::PyRuntimeBatchOutcome>()?;
    m.add_class::<runtime::PyRuntimeObservationReport>()?;
    m.add_class::<runtime::PyRuntimeCancelReport>()?;
    m.add_class::<runtime::PyRuntimeReadOperation>()?;
    m.add_class::<runtime::PyRuntimeWriteOperation>()?;
    m.add_class::<runtime::PyRuntimeObservation>()?;
    m.add_class::<runtime::PyScanOutcome>()?;
    m.add_class::<runtime::PyScanSnapshot>()?;
    m.add_class::<runtime::PyRuntimeScannedObject>()?;
    m.add_class::<runtime::PyRuntimeDeviceScanSnapshot>()?;

    Ok(())
}
