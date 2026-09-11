use std::collections::{BTreeSet, VecDeque};
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_client::client::ReceivedCOVNotification;
use bacnet_client::client::{BACnetClient, RouterInfo};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_client::discovery::IAmEvent;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
#[cfg(feature = "mstp")]
use bacnet_transport::mstp::{MstpConfig as TransportMstpConfig, MstpTransport};
#[cfg(feature = "mstp")]
use bacnet_transport::mstp_serial::{SerialConfig, TokioSerialPort};
#[cfg(feature = "sc")]
use bacnet_transport::sc::{ScReconnectConfig, ScTransport};
#[cfg(feature = "sc")]
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast;

use crate::discovery::router_observation;
use crate::{AttachmentConfig, DevicePath, PropertyRead, RpmBatch, RuntimeError, TransportConfig};
use crate::{
    BbmdSnapshot, BdtRecord, FdtRecord, ForeignDeviceRegistrationStatus, RouterObservation,
};

#[derive(Debug)]
pub(crate) struct AttachmentDiscovery {
    pub(crate) devices: Vec<DiscoveredDevice>,
    pub(crate) routers: Vec<RouterInfo>,
    pub(crate) errors: Vec<RuntimeError>,
}

#[derive(Debug)]
pub(crate) struct AttachmentTopologyResult {
    pub(crate) local_mac: Vec<u8>,
    pub(crate) bbmds: Vec<BbmdSnapshot>,
    pub(crate) routers: Vec<RouterObservation>,
    pub(crate) truncated: bool,
    pub(crate) errors: Vec<RuntimeError>,
}

/// Type-erased runtime client engine. Each variant owns one attachment lifecycle.
pub enum RuntimeTransport {
    /// BACnet/IP client and UDP transport.
    Bip(BACnetClient<BipTransport>),
    /// BACnet MS/TP client and production serial transport.
    #[cfg(feature = "mstp")]
    Mstp(BACnetClient<MstpTransport<TokioSerialPort>>),
    /// BACnet Secure Connect client and production TLS WebSocket transport.
    #[cfg(feature = "sc")]
    Sc(BACnetClient<ScTransport<TlsWebSocket>>),
}

#[cfg(test)]
static BIP_START_COV_HOOK: std::sync::Mutex<Option<(crate::AttachmentId, Vec<u8>)>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn install_bip_start_cov_hook(attachment_id: crate::AttachmentId, apdu: Vec<u8>) {
    let mut hook = BIP_START_COV_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(hook.is_none(), "B/IP start COV hook already installed");
    *hook = Some((attachment_id, apdu));
}

#[cfg(test)]
async fn run_bip_start_cov_hook(
    attachment_id: crate::AttachmentId,
    client: &BACnetClient<BipTransport>,
) {
    let apdu = {
        let mut hook = BIP_START_COV_HOOK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hook.as_ref().is_some_and(|(id, _)| *id == attachment_id) {
            hook.take().map(|(_, apdu)| apdu)
        } else {
            None
        }
    };
    let Some(apdu) = apdu else {
        return;
    };

    let mut observed = client.cov_notifications();
    let local_mac = client.local_mac();
    let destination = SocketAddrV4::new(
        Ipv4Addr::new(local_mac[0], local_mac[1], local_mac[2], local_mac[3]),
        u16::from_be_bytes([local_mac[4], local_mac[5]]),
    );
    let mut frame = Vec::with_capacity(apdu.len() + 6);
    frame.extend_from_slice(&[0x81, 0x0a]);
    frame.extend_from_slice(&u16::try_from(apdu.len() + 6).unwrap().to_be_bytes());
    frame.extend_from_slice(&[0x01, 0x00]);
    frame.extend_from_slice(&apdu);
    let sender = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    sender.send_to(&frame, destination).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), observed.recv())
        .await
        .expect("startup COV was not dispatched before RuntimeTransport::start returned")
        .unwrap();
}

mod configuration;
mod discovery;
mod io;

impl std::fmt::Debug for RuntimeTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bip(_) => f.write_str("RuntimeTransport::Bip(..)"),
            #[cfg(feature = "mstp")]
            Self::Mstp(_) => f.write_str("RuntimeTransport::Mstp(..)"),
            #[cfg(feature = "sc")]
            Self::Sc(_) => f.write_str("RuntimeTransport::Sc(..)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    use crate::{
        AttachmentConfig, AttachmentId, BipConfig, ErrorCode, MstpConfig, ScConfig, TransportConfig,
    };

    use super::RuntimeTransport;

    fn bip(interface: &str) -> AttachmentConfig {
        AttachmentConfig {
            id: AttachmentId::from(1),
            label: "test-bip".to_owned(),
            transport: TransportConfig::Bip(BipConfig {
                interface: interface.to_owned(),
                port: 0,
                broadcast: Some("127.255.255.255".to_owned()),
                bbmd_address: None,
                foreign_device_ttl: None,
            }),
        }
    }

    fn mstp(device: &str) -> AttachmentConfig {
        AttachmentConfig {
            id: AttachmentId::from(9),
            label: "test-mstp".to_owned(),
            transport: TransportConfig::Mstp(MstpConfig {
                device: device.to_owned(),
                baud: 38_400,
                mac: 1,
                max_master: 127,
                max_info_frames: 1,
            }),
        }
    }

    fn sc() -> AttachmentConfig {
        AttachmentConfig {
            id: AttachmentId::from(10),
            label: "test-sc".to_owned(),
            transport: TransportConfig::Sc(ScConfig {
                primary_hub: "wss://primary.example.test".to_owned(),
                failover_hubs: vec!["wss://failover.example.test".to_owned()],
                local_vmac: [1, 2, 3, 4, 5, 6],
                ca_cert: Some("/missing/test-ca.pem".to_owned()),
                client_cert: Some("/missing/test-client.pem".to_owned()),
                client_key: Some("/missing/test-client.key".to_owned()),
                heartbeat_interval_ms: 30_000,
                heartbeat_timeout_ms: 60_000,
                reconnect_initial_delay_ms: 100,
                reconnect_max_delay_ms: 1_000,
                reconnect_max_retries: 3,
            }),
        }
    }

    #[tokio::test]
    async fn bip_factory_preserves_mac_bytes_and_releases_socket_on_stop() {
        let mut transport = RuntimeTransport::start(
            &bip("127.0.0.1"),
            bacnet_client::client::DEFAULT_COV_CHANNEL_CAPACITY,
        )
        .await
        .unwrap();
        let mac = transport.local_mac();
        assert_eq!(&mac[..4], &[127, 0, 0, 1]);
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        assert_ne!(port, 0);

        transport.stop(AttachmentId::from(1)).await.unwrap();
        let rebound = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port));
        assert!(rebound.is_ok(), "B/IP socket was not released: {rebound:?}");
    }

    #[tokio::test]
    async fn factory_rejects_invalid_ip_before_opening_transport() {
        let error = RuntimeTransport::start(
            &bip("definitely-not-an-ip"),
            bacnet_client::client::DEFAULT_COV_CHANNEL_CAPACITY,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidConfig);
        assert_eq!(error.attachment_id, Some(AttachmentId::from(1)));
    }

    #[test]
    fn bip_foreign_device_config_requires_a_valid_paired_address_and_ttl() {
        let invalid_cases = [
            (Some("127.0.0.1:47808"), None),
            (None, Some(60)),
            (Some("127.0.0.1:47808"), Some(0)),
            (Some("not-a-bbmd"), Some(60)),
        ];
        for (address, ttl) in invalid_cases {
            let mut config = bip("127.0.0.1");
            let TransportConfig::Bip(value) = &mut config.transport else {
                unreachable!()
            };
            value.bbmd_address = address.map(str::to_owned);
            value.foreign_device_ttl = ttl;

            let error = RuntimeTransport::validate_config(&config).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidConfig);
            assert_eq!(error.attachment_id, Some(config.id));
        }
    }

    #[test]
    fn bip_foreign_device_config_accepts_socket_address_and_nonzero_ttl() {
        let mut config = bip("127.0.0.1");
        let TransportConfig::Bip(value) = &mut config.transport else {
            unreachable!()
        };
        value.bbmd_address = Some("127.0.0.1:47808".to_owned());
        value.foreign_device_ttl = Some(60);

        RuntimeTransport::validate_config(&config).unwrap();
    }

    #[tokio::test]
    async fn mstp_factory_has_stable_error_and_attachment_context() {
        let config = mstp("/definitely/missing/rusty-bacnet-mstp");
        let error =
            RuntimeTransport::start(&config, bacnet_client::client::DEFAULT_COV_CHANNEL_CAPACITY)
                .await
                .unwrap_err();
        #[cfg(feature = "mstp")]
        assert_eq!(error.code, ErrorCode::SerialUnavailable);
        #[cfg(not(feature = "mstp"))]
        assert_eq!(error.code, ErrorCode::UnsupportedTransport);
        assert_eq!(error.attachment_id, Some(config.id));
    }

    #[test]
    fn mstp_config_is_validated_before_serial_open() {
        let invalid_cases = [
            ("", 38_400, 1, 127, 1),
            ("/dev/null", 115_200, 1, 127, 1),
            ("/dev/null", 38_400, 128, 127, 1),
            ("/dev/null", 38_400, 2, 1, 1),
            ("/dev/null", 38_400, 1, 127, 0),
        ];
        for (device, baud, mac, max_master, max_info_frames) in invalid_cases {
            let mut config = mstp(device);
            let TransportConfig::Mstp(value) = &mut config.transport else {
                unreachable!()
            };
            value.baud = baud;
            value.mac = mac;
            value.max_master = max_master;
            value.max_info_frames = max_info_frames;
            let error = RuntimeTransport::validate_config(&config).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidConfig);
            assert_eq!(error.attachment_id, Some(config.id));
        }
    }

    #[test]
    fn same_mstp_device_must_stop_before_replacement() {
        let current = mstp("/dev/ttyUSB0").transport;
        let mut desired = mstp("/dev/ttyUSB0").transport;
        let TransportConfig::Mstp(value) = &mut desired else {
            unreachable!()
        };
        value.baud = 76_800;
        assert!(RuntimeTransport::must_stop_before_replacement(
            &current, &desired
        ));
        assert!(!RuntimeTransport::must_stop_before_replacement(
            &current,
            &mstp("/dev/ttyUSB1").transport,
        ));
    }

    #[test]
    fn sc_config_rejects_each_invalid_public_boundary() {
        let cases: Vec<Box<dyn FnOnce(&mut ScConfig)>> = vec![
            Box::new(|value| value.primary_hub = "ws://insecure.example.test".to_owned()),
            Box::new(|value| {
                value
                    .failover_hubs
                    .push("wss://extra.example.test".to_owned())
            }),
            Box::new(|value| value.failover_hubs[0] = "https://wrong.example.test".to_owned()),
            Box::new(|value| value.local_vmac = [0; 6]),
            Box::new(|value| value.local_vmac = [0xff; 6]),
            Box::new(|value| value.ca_cert = None),
            Box::new(|value| value.client_key = None),
            Box::new(|value| value.heartbeat_interval_ms = 2_999),
            Box::new(|value| value.heartbeat_timeout_ms = 30_000),
            Box::new(|value| value.reconnect_initial_delay_ms = 0),
            Box::new(|value| value.reconnect_max_delay_ms = 99),
            Box::new(|value| value.reconnect_max_retries = 0),
        ];

        for mutate in cases {
            let mut config = sc();
            let TransportConfig::Sc(value) = &mut config.transport else {
                unreachable!()
            };
            mutate(value);
            let error = RuntimeTransport::validate_config(&config).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidConfig);
            assert_eq!(error.attachment_id, Some(config.id));
        }
    }

    #[cfg(not(feature = "sc"))]
    #[tokio::test]
    async fn sc_factory_has_stable_feature_disabled_error() {
        let config = sc();
        let error =
            RuntimeTransport::start(&config, bacnet_client::client::DEFAULT_COV_CHANNEL_CAPACITY)
                .await
                .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedTransport);
        assert_eq!(error.attachment_id, Some(config.id));
    }
}
