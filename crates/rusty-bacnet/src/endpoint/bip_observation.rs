//! Observation and addressing on the endpoint socket.

use super::*;

#[pymethods]
impl PyBipEndpoint {
    /// Subscribe to raw frames on the endpoint's existing socket, including
    /// malformed received datagrams. A full queue drops new frames; the next
    /// frame queued reports the gap in `Frame.missed_before`. Close ends
    /// every subscription, waking pending reads.
    #[pyo3(signature = (capacity=1024))]
    fn frames<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let lifecycle = self.lifecycle.clone();
        let frames = self.frames.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = lifecycle.session.read().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            let receiver = session
                .frames(capacity)
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            let address = session
                .bip_local_address()
                .and_then(|socket| {
                    crate::frames::local_address(DataLink::Bip, &[], Some(socket.into()))
                })
                .ok_or_else(|| PyRuntimeError::new_err("endpoint not started"))?;
            Ok(frames.subscribe(receiver, crate::frames::FrameContext::local(address), clock))
        })
    }

    /// Subscribe to a record of every I-Am the endpoint's socket receives,
    /// as `BACnetClient.i_am_arrivals` does, leaving out and counting its
    /// own I-Am come back to it: from its own address, of its own device
    /// instance. Close ends every subscription, waking pending reads.
    #[pyo3(signature = (capacity=1024))]
    fn i_am_arrivals<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let lifecycle = self.lifecycle.clone();
        let frames = self.frames.clone();
        let instance = self.config.identity.instance();
        crate::py_async::future_into_py(py, async move {
            let guard = lifecycle.session.read().await;
            let not_started = || PyRuntimeError::new_err("endpoint not started");
            let session = guard.as_ref().ok_or_else(not_started)?;
            let receiver = session.frames(capacity).ok_or_else(not_started)?;
            let own = session.bip_local_address().ok_or_else(not_started)?;
            Ok(crate::i_am_arrivals::subscribe(
                &frames,
                receiver,
                Some((own.into(), instance)),
                clock,
            ))
        })
    }

    /// Raises RuntimeError before startup publication and during/after teardown.
    fn local_address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.lifecycle.session.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = inner.read().await;
            guard
                .as_ref()
                .and_then(|session| session.bip_local_address())
                .map(|address| address.to_string())
                .ok_or_else(|| PyRuntimeError::new_err("endpoint is not active"))
        })
    }
}
