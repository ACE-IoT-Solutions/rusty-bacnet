//! Local admission leases for confirmed COV.

use super::*;
use crate::types::PyRoutedTarget;
use bacnet_client::client::Station;

#[pyclass(name = "ConfirmedCovLease", module = "rusty_bacnet", frozen)]
pub(crate) struct PyConfirmedCovLease(client::ConfirmedCovLease);

#[pymethods]
impl PyConfirmedCovLease {
    /// Withdraw this local expectation, even if the client has already stopped.
    fn close(&self) {
        self.0.close();
    }
}

#[pymethods]
impl BACnetClient {
    /// Register a local expectation before sending SubscribeCOV. Keep the
    /// lease alive while confirmed notifications should be admitted.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, *, initiating_device_instance=None))]
    fn expect_confirmed_cov<'py>(
        &self,
        py: Python<'py>,
        address: &Bound<'py, PyAny>,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        initiating_device_instance: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = match address.cast::<PyRoutedTarget>() {
            Ok(target) => target.get().station(),
            Err(_) => Station::local(
                &PeerAddresses::for_transport(&self.transport_type)
                    .parse(&address.extract::<String>()?)?,
            ),
        };
        let object = monitored_object_identifier.to_rust();
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let lease = started(&inner)
                .await?
                .expect_confirmed_cov(
                    source,
                    subscriber_process_identifier,
                    object,
                    initiating_device_instance,
                )
                .map_err(to_py_err)?;
            Ok(PyConfirmedCovLease(lease))
        })
    }
}
