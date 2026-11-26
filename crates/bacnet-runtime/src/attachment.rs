use bacnet_types::sync::MutexExt as _;
use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::Mutex;

use bacnet_client::client::{BACnetClient, Station};
use bacnet_transport::bip::{BipMode, BipTransport, ForeignBbmd, ForeignDeviceConfig};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;

use crate::config::{AttachmentConfig, TransportConfig};
use crate::health::{
    AttachmentHealth, AttachmentState, ForeignDeviceRegistrationState,
    ForeignDeviceRegistrationStatus,
};
use crate::{DevicePath, PropertyTarget, RuntimeError};

type Client = Box<BACnetClient<BipTransport>>;
#[cfg(test)]
type StopBarrier = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<Result<(), bacnet_types::error::Error>>,
);
type Shutdown =
    Pin<Box<dyn Future<Output = (Client, Result<(), bacnet_types::error::Error>)> + Send>>;

enum Owner {
    Running(Client),
    RetryStop(Client),
    Stopping(Shutdown),
    Stopped,
}

// The permit excludes all other consumers while the owner is outside the slot.
// Drop puts it back before releasing admission, including cancellation mid-request.
struct Lease<'a> {
    slot: &'a Mutex<Owner>,
    owner: Owner,
    _permit: SemaphorePermit<'a>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        *self.slot.locked() = std::mem::replace(&mut self.owner, Owner::Stopped);
    }
}

impl Lease<'_> {
    fn client(&self) -> Result<&Client, RuntimeError> {
        match &self.owner {
            Owner::Running(client) => Ok(client),
            _ => Err(RuntimeError::stopped()),
        }
    }
}

pub(super) struct ManagedAttachment {
    pub(super) config: AttachmentConfig,
    admission: Semaphore,
    owner: Mutex<Owner>,
    local_mac: Vec<u8>,
    status: Mutex<(AttachmentState, Option<crate::ErrorCode>)>,
    #[cfg(test)]
    pub(crate) stop_barrier: Mutex<Option<StopBarrier>>,
}

impl DevicePath {
    fn station(&self) -> Station {
        match self {
            Self::Direct { mac } => Station::local(mac),
            Self::Routed {
                ingress_mac,
                dnet,
                dadr,
            } => Station::routed(ingress_mac, *dnet, dadr),
        }
    }
}

impl ManagedAttachment {
    fn new(config: AttachmentConfig, client: BACnetClient<BipTransport>) -> Self {
        Self {
            config,
            local_mac: client.local_mac().to_vec(),
            owner: Mutex::new(Owner::Running(Box::new(client))),
            admission: Semaphore::new(1),
            status: Mutex::new((AttachmentState::Running, None)),
            #[cfg(test)]
            stop_barrier: Mutex::default(),
        }
    }

    async fn lease(&self) -> Lease<'_> {
        let permit = self
            .admission
            .acquire()
            .await
            .expect("attachment admission stays open");
        let owner = std::mem::replace(&mut *self.owner.locked(), Owner::Stopped);
        Lease {
            slot: &self.owner,
            owner,
            _permit: permit,
        }
    }

    pub(super) async fn health(&self) -> AttachmentHealth {
        let lease = self.lease().await;
        let (state, last_error) = *self.status.locked();
        let local_mac = (!matches!(lease.owner, Owner::Stopped)).then(|| self.local_mac.clone());
        let foreign_device_registration = if matches!(
            &self.config.transport,
            TransportConfig::Bip(crate::BipConfig {
                bbmd_address: Some(_),
                ..
            })
        ) {
            lease
                .client()
                .ok()
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
        let lease = self.lease().await;
        let client = lease.client()?;
        let result = client
            .read_property(
                &target.path.station(),
                target.object_identifier,
                target.property_identifier,
                target.property_array_index,
            )
            .await;
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
        let lease = self.lease().await;
        let client = lease.client()?;
        let result = client
            .write_property(
                &target.path.station(),
                target.object_identifier,
                target.property_identifier,
                target.property_array_index,
                raw_value,
                priority,
            )
            .await;
        result.map_err(|error| RuntimeError::operation(self.config.id, error))
    }

    pub(super) async fn stop(&self, deadline: Option<Instant>) -> Result<(), RuntimeError> {
        *self.status.locked() = (AttachmentState::Stopping, None);
        let stop = self.finish_shutdown();
        let result = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, stop)
                .await
                .unwrap_or_else(|_| {
                    Err(RuntimeError {
                        code: crate::ErrorCode::DeadlineExceeded,
                        retryable: true,
                        attachment_id: Some(self.config.id),
                        message: "attachment shutdown timed out".into(),
                    })
                }),
            None => stop.await,
        };
        *self.status.locked() = match &result {
            Ok(()) => (AttachmentState::Stopped, None),
            Err(error) => (AttachmentState::Failed, Some(error.code)),
        };
        result
    }

    async fn finish_shutdown(&self) -> Result<(), RuntimeError> {
        let mut lease = self.lease().await;
        if matches!(lease.owner, Owner::Running(_) | Owner::RetryStop(_)) {
            let (Owner::Running(mut client) | Owner::RetryStop(mut client)) =
                std::mem::replace(&mut lease.owner, Owner::Stopped)
            else {
                unreachable!()
            };
            #[cfg(test)]
            let barrier = self.stop_barrier.locked().take();
            // Preserve the actual shutdown future: it can own a join removed from the client.
            lease.owner = Owner::Stopping(Box::pin(async move {
                #[cfg(test)]
                if let Some((entered, resume)) = barrier {
                    let _ = entered.send(());
                    if let Ok(Err(error)) = resume.await {
                        return (client, Err(error));
                    }
                }
                let result = client.stop().await;
                (client, result)
            }));
        }
        let Owner::Stopping(shutdown) = &mut lease.owner else {
            return Ok(());
        };
        let (client, result) = shutdown.await;
        match result {
            Ok(()) => {
                lease.owner = Owner::Stopped;
                Ok(())
            }
            Err(error) => {
                lease.owner = Owner::RetryStop(client);
                Err(RuntimeError::start(self.config.id, error))
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
        let bbmd = address.parse::<ForeignBbmd>().map_err(|error| {
            RuntimeError::attachment_config(config.id, format!("invalid BBMD address: {error}"))
        })?;
        builder = builder.mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd,
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
