//! Subscriptions to each configured port's own transport.

use super::{NativeRouter, PyAceMixedRouter, PyRouterPortConfig, bip_endpoint};
use crate::frames::{FrameContext, FrameSubscriptions};
use bacnet_transport::tap::FrameTap;
use pyo3::exceptions::{PyIndexError, PyRuntimeError};
use pyo3::prelude::*;

pub(super) struct RouterFrames {
    ports: Vec<(FrameTap, FrameContext)>,
    pub(super) subscriptions: FrameSubscriptions,
}

impl RouterFrames {
    pub(super) fn new(
        router: &NativeRouter,
        configured: &[PyRouterPortConfig],
        taps: Vec<FrameTap>,
    ) -> Self {
        let ports = taps
            .into_iter()
            .zip(configured)
            .zip(router.ports())
            .map(|((tap, configured), port)| {
                let index = usize::from(port.index());
                let mut context = match configured {
                    PyRouterPortConfig::Bip(_) => FrameContext::local(
                        bip_endpoint(router, index)
                            .expect("started B/IP port address")
                            .to_string(),
                    ),
                    PyRouterPortConfig::Sc(sc) => FrameContext {
                        local_address: crate::frames::hex_address(port.local_mac()).into(),
                        primary_hub: Some(sc.primary_hub.clone()),
                        failover_hub: sc.failover_hub.clone(),
                        ..FrameContext::default()
                    },
                };
                context.port_index = Some(index);
                (tap, context)
            })
            .collect();
        Self {
            ports,
            subscriptions: FrameSubscriptions::default(),
        }
    }
}

#[pymethods]
impl PyAceMixedRouter {
    /// Subscribe to future raw frames on a running configured port. On SC this
    /// includes BVLC-SC controls and encapsulated NPDUs across hub reconnects.
    #[pyo3(signature = (port_index, capacity=1024))]
    fn frames<'py>(
        &self,
        py: Python<'py>,
        port_index: usize,
        capacity: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let lifecycle = self.lifecycle.clone();
        crate::py_async::future_into_py(py, async move {
            lifecycle
                .inspect(|session| {
                    let frames = &session.active().frames;
                    let (tap, context) = frames
                        .ports
                        .get(port_index)
                        .ok_or_else(|| PyIndexError::new_err("router port index out of range"))?;
                    Ok(frames.subscriptions.subscribe(
                        tap.subscribe_with_capacity(capacity),
                        context.clone(),
                        clock,
                    ))
                })
                .await
                .ok_or_else(|| PyRuntimeError::new_err("router not started"))?
        })
    }
}
