use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Subscribe to bounded, passive APDU diagnostics for this lifecycle.
    ///
    /// This unstable API exposes raw NPDU bytes, which may contain sensitive
    /// application data. Lag is reported as BacnetNotificationLagError.
    fn apdu_events(&self) -> PyResult<crate::types::PyApduObserverEventIterator> {
        self.apdu_observer
            .as_ref()
            .ok_or_else(|| {
                PyRuntimeError::new_err(
                    "APDU observer is disabled; construct with apdu_observer=True",
                )
            })?
            .events()
    }
}
