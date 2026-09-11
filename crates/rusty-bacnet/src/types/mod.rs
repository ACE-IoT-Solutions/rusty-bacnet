//! Python-facing type wrappers for BACnet enums, ObjectIdentifier, and PropertyValue.

#![allow(non_snake_case)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Instant;

use bytes::BytesMut;
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::Py;
use tokio::sync::broadcast;

use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_services::common::{BACnetPropertyValue, PropertyReference};
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums as bacnet_enums;
use bacnet_types::primitives;

mod address;
mod apdu_observer;
mod audit;
mod audit_projection;
mod bbmd_control;
mod bvll_management;
mod bvll_policy_control;
mod cov;
mod device;
mod enums;
mod foreign_device;
mod iam;
mod managed_cov;
mod object_identifier;
mod property_value;
mod raw_value;
mod router_info;
mod rpm_wpm;
mod timestamp;

pub use address::{parse_address, PyDirectTarget, PyRoutedTarget, PyTarget};
pub use apdu_observer::{PyApduDecodeError, PyApduObserverEvent, PyApduObserverEventIterator};
pub(crate) use apdu_observer::{PyApduObserverStartGuard, PyApduObserverState};
pub(crate) use audit::{audit_log_query_request_from_py, audit_notification_request_from_py};
pub(crate) use audit_projection::audit_log_query_ack_to_py;
pub(crate) use bbmd_control::{parse_bdt_entries, parse_management_acl, PyBbmdTransportConfig};
pub use bbmd_control::{PyBbmdControl, PyBbmdCounters, PyBbmdSnapshot};
pub use bvll_management::{PyBdtEntry, PyFdtEntry};
pub(crate) use bvll_policy_control::PythonBvllPolicyBridge;
pub use bvll_policy_control::{PyBvllPolicyContext, PyBvllPolicyCounters, PyBvllPolicyVerdict};
pub use cov::{PyCovNotification, PyCovNotificationIterator};
pub use device::PyDiscoveredDevice;
pub use enums::*;
pub use foreign_device::PyForeignDeviceStatus;
pub use iam::{PyIAmEvent, PyIAmEventIterator};
pub(crate) use managed_cov::PyManagedCOVTarget;
pub use managed_cov::{PyManagedCOVEvent, PyManagedCOVEventIterator, PyManagedCOVSubscription};
pub use object_identifier::PyObjectIdentifier;
pub use property_value::PyPropertyValue;
pub use raw_value::{decode_raw_value, describe_tags, PyRawTag};
pub use router_info::PyRouterInfo;
pub(crate) use rpm_wpm::{
    decode_complete_property_value, py_to_rpm_specs, py_to_wpm_specs, rpm_ack_to_py,
};
pub use timestamp::PyBACnetTimeStamp;

// Module registration
// ---------------------------------------------------------------------------

/// Register all type classes with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBbmdControl>()?;
    m.add_class::<PyBbmdCounters>()?;
    m.add_class::<PyBbmdSnapshot>()?;
    m.add_class::<PyBvllPolicyContext>()?;
    m.add_class::<PyBvllPolicyCounters>()?;
    m.add_class::<PyBvllPolicyVerdict>()?;
    // Enum types — add class then populate constants from ALL_NAMED.
    m.add_class::<PyObjectType>()?;
    PyObjectType::register_constants(&m.getattr("ObjectType")?)?;

    m.add_class::<PyPropertyIdentifier>()?;
    PyPropertyIdentifier::register_constants(&m.getattr("PropertyIdentifier")?)?;

    m.add_class::<PyErrorClass>()?;
    PyErrorClass::register_constants(&m.getattr("ErrorClass")?)?;

    m.add_class::<PyErrorCode>()?;
    PyErrorCode::register_constants(&m.getattr("ErrorCode")?)?;

    m.add_class::<PyAuditOperation>()?;
    PyAuditOperation::register_constants(&m.getattr("AuditOperation")?)?;

    m.add_class::<PyEnableDisable>()?;
    PyEnableDisable::register_constants(&m.getattr("EnableDisable")?)?;

    m.add_class::<PyReinitializedState>()?;
    PyReinitializedState::register_constants(&m.getattr("ReinitializedState")?)?;

    m.add_class::<PySegmentation>()?;
    PySegmentation::register_constants(&m.getattr("Segmentation")?)?;

    m.add_class::<PyLifeSafetyOperation>()?;
    PyLifeSafetyOperation::register_constants(&m.getattr("LifeSafetyOperation")?)?;

    m.add_class::<PyEventState>()?;
    PyEventState::register_constants(&m.getattr("EventState")?)?;

    m.add_class::<PyEnrollmentSummaryEventStateFilter>()?;
    PyEnrollmentSummaryEventStateFilter::register_constants(
        &m.getattr("EnrollmentSummaryEventStateFilter")?,
    )?;

    m.add_class::<PyEventType>()?;
    PyEventType::register_constants(&m.getattr("EventType")?)?;

    m.add_class::<PyMessagePriority>()?;
    PyMessagePriority::register_constants(&m.getattr("MessagePriority")?)?;

    // Composite types
    m.add_class::<PyObjectIdentifier>()?;
    m.add_class::<PyPropertyValue>()?;
    m.add_class::<PyBACnetTimeStamp>()?;
    m.add_class::<PyDiscoveredDevice>()?;
    m.add_class::<PyCovNotification>()?;
    m.add_class::<PyCovNotificationIterator>()?;
    m.add_class::<PyForeignDeviceStatus>()?;
    m.add_class::<PyIAmEvent>()?;
    m.add_class::<PyIAmEventIterator>()?;
    m.add_class::<PyManagedCOVEvent>()?;
    m.add_class::<PyManagedCOVEventIterator>()?;
    m.add_class::<PyManagedCOVSubscription>()?;
    m.add_class::<PyBdtEntry>()?;
    m.add_class::<PyFdtEntry>()?;
    m.add_class::<PyRouterInfo>()?;
    m.add_class::<PyApduDecodeError>()?;
    m.add_class::<PyApduObserverEvent>()?;
    m.add_class::<PyApduObserverEventIterator>()?;
    m.add_class::<PyDirectTarget>()?;
    m.add_class::<PyRoutedTarget>()?;
    m.add_class::<PyRawTag>()?;
    m.add_function(wrap_pyfunction!(decode_raw_value, m)?)?;
    m.add_function(wrap_pyfunction!(describe_tags, m)?)?;

    Ok(())
}

#[cfg(test)]
mod tests;
