use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::{BipMode, BipTransport, ForeignDeviceConfig};
use tokio::sync::Mutex;

use crate::config::{AttachmentConfig, TransportConfig};
use crate::health::{
    AttachmentHealth, AttachmentState, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus,
};
use crate::{DevicePath, PropertyTarget, RuntimeError};

pub(super) struct ManagedAttachment {
    pub(super) config: AttachmentConfig,
    pub(super) client: Mutex<Option<BACnetClient<BipTransport>>>,
    state: Mutex<AttachmentState>,
    last_error: Mutex<Option<crate::ErrorCode>>,
}

impl ManagedAttachment {
    fn new(config: AttachmentConfig, client: BACnetClient<BipTransport>) -> Self {
        Self {
            config,
            client: Mutex::new(Some(client)),
            state: Mutex::new(AttachmentState::Running),
            last_error: Mutex::new(None),
        }
    }

    pub(super) async fn health(&self) -> AttachmentHealth {
        let state = *self.state.lock().await;
        let last_error = *self.last_error.lock().await;
        let local_mac = self
            .client
            .lock()
            .await
            .as_ref()
            .map(|client| client.local_mac().to_vec());
        let foreign_device_registration = if matches!(
            &self.config.transport,
            TransportConfig::Bip(crate::BipConfig {
                bbmd_address: Some(_),
                ..
            })
        ) {
            self.client
                .lock()
                .await
                .as_ref()
                .and_then(|client| client.foreign_registration().ok())
                .map(ForeignDeviceRegistrationStatus::from)
        } else {
            None
        };
        let last_error = foreign_device_registration
            .as_ref()
            .filter(|registration| {
                matches!(
                    registration.state,
                    ForeignDeviceRegistrationState::Rejected
                        | ForeignDeviceRegistrationState::Expired
                )
            })
            .map(|_| crate::ErrorCode::ForeignDeviceRegistrationFailed)
            .or(last_error);
        AttachmentHealth {
            id: self.config.id,
            label: self.config.label.clone(),
            state,
            last_error,
            local_mac,
            foreign_device_registration,
        }
    }

    pub(super) async fn read_property(
        &self,
        target: &PropertyTarget,
    ) -> Result<Vec<u8>, RuntimeError> {
        let client = self.client.lock().await;
        let client = client.as_ref().ok_or_else(RuntimeError::stopped)?;
        let result = match &target.path {
            DevicePath::Direct { mac } => {
                client
                    .read_property(
                        mac,
                        target.object_identifier,
                        target.property_identifier,
                        target.property_array_index,
                    )
                    .await
            }
            DevicePath::Routed {
                ingress_mac,
                dnet,
                dadr,
            } => {
                client
                    .read_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        target.object_identifier,
                        target.property_identifier,
                        target.property_array_index,
                    )
                    .await
            }
        };
        result
            .map(|ack| ack.property_value)
            .map_err(|error| RuntimeError::operation(self.config.id, error))
    }

    pub(super) async fn write_property(
        &self,
        target: &PropertyTarget,
        raw_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), RuntimeError> {
        let client = self.client.lock().await;
        let client = client.as_ref().ok_or_else(RuntimeError::stopped)?;
        let result = match &target.path {
            DevicePath::Direct { mac } => {
                client
                    .write_property(
                        mac,
                        target.object_identifier,
                        target.property_identifier,
                        target.property_array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
            DevicePath::Routed {
                ingress_mac,
                dnet,
                dadr,
            } => {
                client
                    .write_property_routed(
                        ingress_mac,
                        *dnet,
                        dadr,
                        target.object_identifier,
                        target.property_identifier,
                        target.property_array_index,
                        raw_value,
                        priority,
                    )
                    .await
            }
        };
        result.map_err(|error| RuntimeError::operation(self.config.id, error))
    }

    pub(super) async fn stop(&self, timeout: Duration) -> Result<(), RuntimeError> {
        *self.state.lock().await = AttachmentState::Stopping;
        let id = self.config.id;
        let stop = async {
            let mut client = self.client.lock().await.take();
            let Some(mut client) = client.take() else {
                return Ok(());
            };
            client.stop().await
        };
        match tokio::time::timeout(timeout, stop).await {
            Ok(Ok(())) => {
                *self.state.lock().await = AttachmentState::Stopped;
                Ok(())
            }
            Ok(Err(error)) => {
                let runtime_error = RuntimeError::start(id, error);
                *self.last_error.lock().await = Some(runtime_error.code);
                *self.state.lock().await = AttachmentState::Failed;
                Err(runtime_error)
            }
            Err(_) => {
                let runtime_error = RuntimeError {
                    code: crate::ErrorCode::DeadlineExceeded,
                    retryable: true,
                    attachment_id: Some(id),
                    message: "attachment shutdown timed out".to_owned(),
                };
                *self.last_error.lock().await = Some(runtime_error.code);
                *self.state.lock().await = AttachmentState::Failed;
                Err(runtime_error)
            }
        }
    }
}

pub(super) async fn start_attachment(
    config: AttachmentConfig,
) -> Result<ManagedAttachment, RuntimeError> {
    let bip = match &config.transport {
        TransportConfig::Bip(config) => config,
        TransportConfig::Mstp(_) => return Err(RuntimeError::unsupported(config.id, "MS/TP")),
        TransportConfig::Sc(_) => return Err(RuntimeError::unsupported(config.id, "BACnet/SC")),
    };
    let interface = bip.interface.parse::<Ipv4Addr>().map_err(|error| {
        RuntimeError::attachment_config(config.id, format!("invalid B/IP interface: {error}"))
    })?;
    let broadcast = bip
        .broadcast
        .as_deref()
        .unwrap_or("255.255.255.255")
        .parse::<Ipv4Addr>()
        .map_err(|error| {
            RuntimeError::attachment_config(config.id, format!("invalid B/IP broadcast: {error}"))
        })?;
    let mut builder = BACnetClient::bip_builder()
        .interface(interface)
        .port(bip.port)
        .broadcast_address(broadcast);
    if let (Some(address), Some(ttl)) = (&bip.bbmd_address, bip.foreign_device_ttl) {
        let bbmd = address.parse::<SocketAddrV4>().map_err(|error| {
            RuntimeError::attachment_config(config.id, format!("invalid BBMD address: {error}"))
        })?;
        builder = builder.mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: *bbmd.ip(),
            bbmd_port: bbmd.port(),
            ttl,
            renewal_interval: None,
        }));
    }
    let client = builder
        .build()
        .await
        .map_err(|error| RuntimeError::start(config.id, error))?;
    Ok(ManagedAttachment::new(config, client))
}
