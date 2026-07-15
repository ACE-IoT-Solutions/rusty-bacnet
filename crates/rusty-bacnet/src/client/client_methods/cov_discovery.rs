use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // COV
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None))]
    #[allow(clippy::too_many_arguments)]
    fn subscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: PyTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (mac, routing) = address.into_parts()?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            if let Some((dnet, dadr)) = routing {
                c.subscribe_cov_routed(
                    &mac,
                    dnet,
                    &dadr,
                    subscriber_process_identifier,
                    oid,
                    confirmed,
                    lifetime,
                )
                .await
            } else {
                c.subscribe_cov(
                    &mac,
                    subscriber_process_identifier,
                    oid,
                    confirmed,
                    lifetime,
                )
                .await
            }
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Unsubscribe from COV notifications for an object.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier))]
    fn unsubscribe_cov<'py>(
        &self,
        py: Python<'py>,
        address: PyTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = monitored_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (mac, routing) = address.into_parts()?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            if let Some((dnet, dadr)) = routing {
                c.unsubscribe_cov_routed(&mac, dnet, &dadr, subscriber_process_identifier, oid)
                    .await
            } else {
                c.unsubscribe_cov(&mac, subscriber_process_identifier, oid)
                    .await
            }
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Start a finite COV subscription with automatic renewal and explicit cancellation.
    #[pyo3(signature = (address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime, renewal_margin_ms=30000, event_channel_capacity=16))]
    #[allow(clippy::too_many_arguments)]
    fn manage_cov_subscription<'py>(
        &self,
        py: Python<'py>,
        address: PyTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: PyObjectIdentifier,
        confirmed: bool,
        lifetime: u32,
        renewal_margin_ms: u64,
        event_channel_capacity: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let managed_cov = Arc::clone(&self.managed_cov);
        let oid = monitored_object_identifier.to_rust();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (mac, routing) = address.into_parts()?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let options = client::ManagedCOVSubscriptionOptions::default()
                .with_renewal_margin(std::time::Duration::from_millis(renewal_margin_ms))
                .with_event_channel_capacity(event_channel_capacity);
            let (managed, target) = if let Some((network, address)) = routing {
                let managed = Arc::clone(&c)
                    .manage_cov_subscription_routed(
                        &mac,
                        network,
                        &address,
                        subscriber_process_identifier,
                        oid,
                        confirmed,
                        lifetime,
                        options,
                    )
                    .await
                    .map_err(to_py_err)?;
                (
                    managed,
                    PyManagedCOVTarget::Routed {
                        router_mac: mac,
                        network,
                        address,
                    },
                )
            } else {
                let managed = Arc::clone(&c)
                    .manage_cov_subscription(
                        &mac,
                        subscriber_process_identifier,
                        oid,
                        confirmed,
                        lifetime,
                        options,
                    )
                    .await
                    .map_err(to_py_err)?;
                (managed, PyManagedCOVTarget::Direct(mac))
            };
            let handle = PyManagedCOVSubscription::new(
                managed,
                Arc::downgrade(&c),
                target,
                subscriber_process_identifier,
                oid,
            );
            managed_cov
                .lock()
                .map_err(|_| PyRuntimeError::new_err("managed COV registry lock poisoned"))?
                .push(Arc::downgrade(&handle.managed));
            Ok(handle)
        })
    }

    /// Get an async iterator yielding incoming COV notifications.
    fn cov_notifications<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = inner.lock().await;
            let c = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("client not started — use 'async with'"))?;
            let rx = c.cov_notifications();
            Ok(PyCovNotificationIterator::new(rx))
        })
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Discover routers and merge I-Am-Router-To-Network claims by responder.
    #[pyo3(signature = (network=None, timeout_ms=1000))]
    fn who_is_router<'py>(
        &self,
        py: Python<'py>,
        network: Option<u16>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let routers = c
                .who_is_router_to_network(network, std::time::Duration::from_millis(timeout_ms))
                .await
                .map_err(to_py_err)?;
            Ok(routers
                .into_iter()
                .map(PyRouterInfo::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Return the latest router snapshot, including partial results retained
    /// when a caller cancels an in-progress discovery observation.
    fn router_snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            Ok(c.router_snapshot()
                .await
                .into_iter()
                .map(PyRouterInfo::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Send a WhoHas broadcast to find an object by identifier.
    #[pyo3(signature = (object_id, low_limit=None, high_limit=None))]
    fn who_has_by_id<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_has(WhoHasObject::Identifier(oid), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send a WhoHas broadcast to find an object by name.
    #[pyo3(signature = (name, low_limit=None, high_limit=None))]
    fn who_has_by_name<'py>(
        &self,
        py: Python<'py>,
        name: String,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.who_has(WhoHasObject::Name(name), low_limit, high_limit)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get a list of all discovered devices.
    fn discovered_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let devices = c.discovered_devices().await;
            Ok(devices
                .into_iter()
                .map(PyDiscoveredDevice::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Look up a discovered device by instance number.
    fn get_device<'py>(&self, py: Python<'py>, instance: u32) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            Ok(c.get_device(instance)
                .await
                .map(PyDiscoveredDevice::from_rust))
        })
    }

    /// Clear the discovered devices table.
    fn clear_devices<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.clear_devices().await;
            Ok(())
        })
    }
}
