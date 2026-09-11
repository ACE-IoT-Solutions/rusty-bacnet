use super::super::*;
use crate::types::{PyBdtEntry, PyFdtEntry};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

#[pymethods]
impl BACnetClient {
    /// Read a BBMD Broadcast Distribution Table through an isolated B/IP probe.
    #[pyo3(signature = (address, timeout_ms=3000))]
    fn read_bdt<'py>(
        &self,
        py: Python<'py>,
        address: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let config = self.management_config(&address)?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let entries = run_management_probe(config, timeout_ms, |transport, target| {
                Box::pin(async move { transport.read_bdt(target).await })
            })
            .await
            .map_err(to_py_err)?;
            Ok(entries
                .into_iter()
                .map(PyBdtEntry::from_rust)
                .collect::<Vec<_>>())
        })
    }

    /// Read a BBMD Foreign Device Table through an isolated B/IP probe.
    #[pyo3(signature = (address, timeout_ms=3000))]
    fn read_fdt<'py>(
        &self,
        py: Python<'py>,
        address: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let config = self.management_config(&address)?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let entries = run_management_probe(config, timeout_ms, |transport, target| {
                Box::pin(async move { transport.read_fdt(target).await })
            })
            .await
            .map_err(to_py_err)?;
            Ok(entries
                .into_iter()
                .map(PyFdtEntry::from_rust)
                .collect::<Vec<_>>())
        })
    }
}

impl BACnetClient {
    fn management_config(&self, address: &str) -> PyResult<ManagementConfig> {
        if self.transport_type != "bip" {
            return Err(to_py_err(Error::Encoding(format!(
                "BVLC management requires transport='bip', got '{}'",
                self.transport_type
            ))));
        }
        let interface = self
            .interface
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid interface: {e}")))?;
        let broadcast = self
            .broadcast_address
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid broadcast address: {e}")))?;
        let (target_ip, target_port) = address.rsplit_once(':').ok_or_else(|| {
            PyValueError::new_err(
                "BBMD address must be an IPv4 BACnet/IP address in 'ip:port' form",
            )
        })?;
        let target_ip: Ipv4Addr = target_ip.parse().map_err(|_| {
            PyValueError::new_err(
                "BBMD address must be an IPv4 BACnet/IP address in 'ip:port' form",
            )
        })?;
        if target_ip.is_unspecified() || target_ip.is_multicast() || target_ip.is_broadcast() {
            return Err(PyValueError::new_err("BBMD address must be unicast"));
        }
        let target_port: u16 = target_port
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid BBMD port: {e}")))?;
        if target_port == 0 {
            return Err(PyValueError::new_err("BBMD port must be in 1..=65535"));
        }
        let mut target = target_ip.octets().to_vec();
        target.extend_from_slice(&target_port.to_be_bytes());
        Ok(ManagementConfig {
            interface,
            broadcast,
            target,
        })
    }
}

struct ManagementConfig {
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    target: Vec<u8>,
}

async fn run_management_probe<R, F>(
    config: ManagementConfig,
    timeout_ms: u64,
    operation: F,
) -> Result<R, Error>
where
    F: for<'a> FnOnce(
        &'a BipTransport,
        &'a [u8],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<R, Error>> + Send + 'a>,
    >,
{
    let mut transport = BipTransport::new(config.interface, 0, config.broadcast);
    let _rx = transport.start().await?;
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let result = match tokio::time::timeout(timeout, operation(&transport, &config.target)).await {
        Ok(result) => result,
        Err(_) => Err(Error::Timeout(timeout)),
    };
    let stop_result = transport.stop().await;
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            stop_result?;
            Ok(value)
        }
    }
}
