use super::*;

/// Typed B/IP runtime attachment configuration.
#[pyclass(name = "RuntimeAttachment", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeAttachment {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) label: String,
    #[pyo3(get)]
    pub(super) interface: String,
    #[pyo3(get)]
    pub(super) port: u16,
    #[pyo3(get)]
    pub(super) broadcast: Option<String>,
    #[pyo3(get)]
    pub(super) bbmd_address: Option<String>,
    #[pyo3(get)]
    pub(super) foreign_device_ttl: Option<u16>,
}

#[pymethods]
impl PyRuntimeAttachment {
    #[new]
    #[pyo3(signature = (attachment_id, label, interface, port=47808, broadcast=None, bbmd_address=None, foreign_device_ttl=None))]
    fn new(
        attachment_id: u128,
        label: String,
        interface: String,
        port: u16,
        broadcast: Option<String>,
        bbmd_address: Option<String>,
        foreign_device_ttl: Option<u16>,
    ) -> Self {
        Self {
            attachment_id,
            label,
            interface,
            port,
            broadcast,
            bbmd_address,
            foreign_device_ttl,
        }
    }
}

/// Typed MS/TP runtime attachment configuration.
#[pyclass(name = "RuntimeMstpAttachment", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeMstpAttachment {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) label: String,
    #[pyo3(get)]
    pub(super) device: String,
    #[pyo3(get)]
    pub(super) baud: u32,
    #[pyo3(get)]
    pub(super) mac: u8,
    #[pyo3(get)]
    pub(super) max_master: u8,
    #[pyo3(get)]
    pub(super) max_info_frames: u8,
}

#[pymethods]
impl PyRuntimeMstpAttachment {
    #[new]
    #[pyo3(signature = (attachment_id, label, device, baud=38400, mac=0, max_master=127, max_info_frames=1))]
    fn new(
        attachment_id: u128,
        label: String,
        device: String,
        baud: u32,
        mac: u8,
        max_master: u8,
        max_info_frames: u8,
    ) -> Self {
        Self {
            attachment_id,
            label,
            device,
            baud,
            mac,
            max_master,
            max_info_frames,
        }
    }
}

/// Typed BACnet/SC runtime attachment configuration.
#[pyclass(name = "RuntimeScAttachment", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeScAttachment {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) label: String,
    #[pyo3(get)]
    pub(super) primary_hub: String,
    #[pyo3(get)]
    pub(super) failover_hub: Option<String>,
    #[pyo3(get)]
    pub(super) local_vmac: Vec<u8>,
    #[pyo3(get)]
    pub(super) ca_cert: Option<String>,
    #[pyo3(get)]
    pub(super) client_cert: Option<String>,
    #[pyo3(get)]
    pub(super) client_key: Option<String>,
    #[pyo3(get)]
    pub(super) heartbeat_interval_ms: u64,
    #[pyo3(get)]
    pub(super) heartbeat_timeout_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_initial_delay_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_max_delay_ms: u64,
    #[pyo3(get)]
    pub(super) reconnect_max_retries: u32,
}

#[pymethods]
impl PyRuntimeScAttachment {
    #[new]
    #[pyo3(signature = (attachment_id, label, primary_hub, local_vmac, ca_cert, client_cert, client_key, failover_hub=None, heartbeat_interval_ms=30000, heartbeat_timeout_ms=60000, reconnect_initial_delay_ms=10000, reconnect_max_delay_ms=600000, reconnect_max_retries=10))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        attachment_id: u128,
        label: String,
        primary_hub: String,
        local_vmac: Vec<u8>,
        ca_cert: String,
        client_cert: String,
        client_key: String,
        failover_hub: Option<String>,
        heartbeat_interval_ms: u64,
        heartbeat_timeout_ms: u64,
        reconnect_initial_delay_ms: u64,
        reconnect_max_delay_ms: u64,
        reconnect_max_retries: u32,
    ) -> PyResult<Self> {
        if local_vmac.len() != 6 {
            return Err(PyValueError::new_err("local_vmac must be exactly 6 bytes"));
        }
        Ok(Self {
            attachment_id,
            label,
            primary_hub,
            failover_hub,
            local_vmac,
            ca_cert: Some(ca_cert),
            client_cert: Some(client_cert),
            client_key: Some(client_key),
            heartbeat_interval_ms,
            heartbeat_timeout_ms,
            reconnect_initial_delay_ms,
            reconnect_max_delay_ms,
            reconnect_max_retries,
        })
    }
}

#[derive(FromPyObject)]
pub(super) enum PyRuntimeAttachmentConfig {
    Bip(PyRuntimeAttachment),
    Mstp(PyRuntimeMstpAttachment),
    Sc(PyRuntimeScAttachment),
}

impl PyRuntimeAttachmentConfig {
    pub(super) fn into_rust(self) -> AttachmentConfig {
        match self {
            Self::Bip(attachment) => AttachmentConfig {
                id: AttachmentId::from(attachment.attachment_id),
                label: attachment.label,
                transport: TransportConfig::Bip(BipConfig {
                    interface: attachment.interface,
                    port: attachment.port,
                    broadcast: attachment.broadcast,
                    bbmd_address: attachment.bbmd_address,
                    foreign_device_ttl: attachment.foreign_device_ttl,
                }),
            },
            Self::Mstp(attachment) => AttachmentConfig {
                id: AttachmentId::from(attachment.attachment_id),
                label: attachment.label,
                transport: TransportConfig::Mstp(MstpConfig {
                    device: attachment.device,
                    baud: attachment.baud,
                    mac: attachment.mac,
                    max_master: attachment.max_master,
                    max_info_frames: attachment.max_info_frames,
                }),
            },
            Self::Sc(attachment) => {
                let mut local_vmac = [0; 6];
                local_vmac.copy_from_slice(&attachment.local_vmac);
                AttachmentConfig {
                    id: AttachmentId::from(attachment.attachment_id),
                    label: attachment.label,
                    transport: TransportConfig::Sc(ScConfig {
                        primary_hub: attachment.primary_hub,
                        failover_hubs: attachment.failover_hub.into_iter().collect(),
                        local_vmac,
                        ca_cert: attachment.ca_cert,
                        client_cert: attachment.client_cert,
                        client_key: attachment.client_key,
                        heartbeat_interval_ms: attachment.heartbeat_interval_ms,
                        heartbeat_timeout_ms: attachment.heartbeat_timeout_ms,
                        reconnect_initial_delay_ms: attachment.reconnect_initial_delay_ms,
                        reconnect_max_delay_ms: attachment.reconnect_max_delay_ms,
                        reconnect_max_retries: attachment.reconnect_max_retries,
                    }),
                }
            }
        }
    }
}

/// Result of one revisioned native attachment reconciliation.
#[pyclass(name = "RuntimeReconcileReport", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PyRuntimeReconcileReport {
    #[pyo3(get)]
    pub(super) revision: u64,
    #[pyo3(get)]
    pub(super) generation: u64,
    #[pyo3(get)]
    pub(super) added: Vec<u128>,
    #[pyo3(get)]
    pub(super) updated: Vec<u128>,
    #[pyo3(get)]
    pub(super) removed: Vec<u128>,
    #[pyo3(get)]
    pub(super) idempotent: bool,
}

/// One persisted attachment-qualified direct or routed device path.
#[pyclass(name = "RuntimePersistedDevice", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyRuntimePersistedDevice {
    #[pyo3(get)]
    pub(super) attachment_id: u128,
    #[pyo3(get)]
    pub(super) device_instance: u32,
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
}

#[pymethods]
impl PyRuntimePersistedDevice {
    #[new]
    #[pyo3(signature = (attachment_id, device_instance, path_mac, vendor_id=0, max_apdu_length=1476, routed_dnet=None, routed_dadr=None))]
    fn new(
        attachment_id: u128,
        device_instance: u32,
        path_mac: Vec<u8>,
        vendor_id: u16,
        max_apdu_length: u16,
        routed_dnet: Option<u16>,
        routed_dadr: Option<Vec<u8>>,
    ) -> PyResult<Self> {
        if routed_dnet.is_some() != routed_dadr.is_some() {
            return Err(PyValueError::new_err(
                "routed_dnet and routed_dadr must be provided together",
            ));
        }
        Ok(Self {
            attachment_id,
            device_instance,
            path_mac,
            routed_dnet,
            routed_dadr,
            vendor_id,
            max_apdu_length,
        })
    }
}

impl PyRuntimePersistedDevice {
    pub(super) fn into_rust(self) -> PersistedDevice {
        let path = match (self.routed_dnet, self.routed_dadr) {
            (Some(dnet), Some(dadr)) => DevicePath::Routed {
                ingress_mac: self.path_mac,
                dnet,
                dadr,
            },
            (None, None) => DevicePath::Direct { mac: self.path_mac },
            _ => unreachable!("constructor enforces paired routed fields"),
        };
        PersistedDevice {
            key: DeviceKey {
                attachment_id: AttachmentId::from(self.attachment_id),
                device_instance: self.device_instance,
            },
            path,
            vendor_id: self.vendor_id,
            max_apdu_length: self.max_apdu_length,
        }
    }
}
