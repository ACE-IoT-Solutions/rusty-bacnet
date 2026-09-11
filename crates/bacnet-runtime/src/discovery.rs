use std::time::Duration;

use bacnet_client::client::RouterInfo;
use bacnet_client::discovery::DiscoveredDevice;

use crate::{
    AttachmentId, DeviceKey, DeviceObservation, DevicePath, DeviceSelection, RuntimeError,
};

/// Coarse multi-attachment discovery request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryRequest {
    /// Selected attachments; empty means every configured attachment.
    pub attachment_ids: Vec<AttachmentId>,
    /// Optional inclusive lower Device-instance limit.
    pub low_limit: Option<u32>,
    /// Optional inclusive upper Device-instance limit.
    pub high_limit: Option<u32>,
    /// Time to collect I-Am and router announcements.
    pub observation_window: Duration,
    /// Clear each selected client table before sending Who-Is.
    pub clear_existing: bool,
    /// Collect I-Am-Router-To-Network announcements in the same operation.
    pub collect_routers: bool,
    /// Optional network for a scoped router query; `None` requests all networks.
    pub router_network: Option<u16>,
}

impl Default for DiscoveryRequest {
    fn default() -> Self {
        Self {
            attachment_ids: Vec::new(),
            low_limit: None,
            high_limit: None,
            observation_window: Duration::from_secs(3),
            clear_existing: false,
            collect_routers: true,
            router_network: None,
        }
    }
}

/// One router observation with attachment and lossless path provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouterObservation {
    /// Attachment on which the router announcement arrived.
    pub attachment_id: AttachmentId,
    /// Direct or routed source path.
    pub path: DevicePath,
    /// Sorted networks advertised by the router.
    pub networks: Vec<u16>,
}

/// Coarse discovery result across selected attachments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverySnapshot {
    /// Runtime generation used for the operation.
    pub generation: u64,
    /// Device-index revision after merging results.
    pub index_revision: u64,
    /// One selection record per observed device instance.
    pub devices: Vec<DeviceSelection>,
    /// Router observations from all successful attachments.
    pub routers: Vec<RouterObservation>,
    /// Per-attachment failures; successful attachments remain represented.
    pub errors: Vec<RuntimeError>,
}

pub(crate) fn device_observation(
    attachment_id: AttachmentId,
    device: DiscoveredDevice,
) -> DeviceObservation {
    let path = match (device.source_network, device.source_address) {
        (Some(dnet), Some(dadr)) => DevicePath::Routed {
            ingress_mac: device.mac_address.to_vec(),
            dnet,
            dadr: dadr.to_vec(),
        },
        _ => DevicePath::Direct {
            mac: device.mac_address.to_vec(),
        },
    };
    DeviceObservation {
        key: DeviceKey {
            attachment_id,
            device_instance: device.object_identifier.instance_number(),
        },
        path,
        vendor_id: device.vendor_id,
        max_apdu_length: u16::try_from(device.max_apdu_length).unwrap_or(u16::MAX),
        revision: 0,
    }
}

pub(crate) fn router_observation(
    attachment_id: AttachmentId,
    router: RouterInfo,
) -> RouterObservation {
    let path = match router.source_network {
        Some(source) => DevicePath::Routed {
            ingress_mac: router.source_mac.to_vec(),
            dnet: source.network,
            dadr: source.mac_address.to_vec(),
        },
        None => DevicePath::Direct {
            mac: router.source_mac.to_vec(),
        },
    };
    RouterObservation {
        attachment_id,
        path,
        networks: router.networks,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bacnet_client::client::RouterInfo;
    use bacnet_client::discovery::DiscoveredDevice;
    use bacnet_encoding::npdu::NpduAddress;
    use bacnet_types::enums::{ObjectType, Segmentation};
    use bacnet_types::primitives::ObjectIdentifier;
    use bacnet_types::MacAddr;

    use crate::{AttachmentId, DevicePath};

    use super::{device_observation, router_observation};

    #[test]
    fn routed_device_mapping_preserves_ingress_and_every_dadr_byte() {
        let mapped = device_observation(
            AttachmentId::from(1),
            DiscoveredDevice {
                object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
                mac_address: MacAddr::from_slice(&[198, 51, 100, 7, 0xba, 0xc2]),
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::BOTH,
                max_segments_accepted: None,
                vendor_id: 42,
                last_seen: Instant::now(),
                source_network: Some(2200),
                source_address: Some(MacAddr::from_slice(&[0, 0x7f, 1])),
            },
        );
        assert_eq!(
            mapped.path,
            DevicePath::Routed {
                ingress_mac: vec![198, 51, 100, 7, 0xba, 0xc2],
                dnet: 2200,
                dadr: vec![0, 0x7f, 1],
            }
        );
    }

    #[test]
    fn routed_router_mapping_retains_source_path_and_networks() {
        let mapped = router_observation(
            AttachmentId::from(2),
            RouterInfo {
                source_mac: MacAddr::from_slice(&[192, 0, 2, 1, 0xba, 0xc0]),
                source_network: Some(NpduAddress {
                    network: 1001,
                    mac_address: MacAddr::from_slice(&[0, 5]),
                }),
                networks: vec![2001, 2002],
            },
        );
        assert_eq!(
            mapped.path,
            DevicePath::Routed {
                ingress_mac: vec![192, 0, 2, 1, 0xba, 0xc0],
                dnet: 1001,
                dadr: vec![0, 5],
            }
        );
        assert_eq!(mapped.networks, vec![2001, 2002]);
    }
}
