//! Server-owned publication of live B/IP state to NetworkPort objects.

use std::sync::Arc;
use std::time::Duration;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::network_port::{
    network_port_snapshot_channel, NetworkPortLiveSnapshot, NetworkPortSnapshotPublisher,
};
use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};
use bacnet_transport::bip::{BbmdSnapshot, BbmdSnapshotReader};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{IPMode, NetworkType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

const PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct BoundPort {
    publisher: NetworkPortSnapshotPublisher,
    baseline: NetworkPortLiveSnapshot,
}

/// Owns every write capability paired with a database-bound reader.
pub(super) struct NetworkPortLiveController {
    ports: Vec<BoundPort>,
    bbmd: Option<BbmdSnapshotReader>,
    task: Option<JoinHandle<()>>,
}

impl NetworkPortLiveController {
    /// Bind readers while the database is still exclusively owned. Only IPv4
    /// NetworkPort objects are associated with the concrete B/IP transport.
    pub(super) fn bind<T: TransportPort + 'static>(db: &mut ObjectDatabase, transport: &T) -> Self {
        let Some(observation) = transport.bip_network_port_observation() else {
            return Self::empty();
        };

        let mut ports = Vec::new();
        for oid in db.find_by_type(ObjectType::NETWORK_PORT) {
            let Some(object) = db.get_mut(&oid) else {
                continue;
            };
            let Some(baseline) = baseline_snapshot(object.as_ref()) else {
                continue;
            };
            if baseline.network_type != NetworkType::IPV4.to_raw() {
                continue;
            }
            let (reader, publisher) = network_port_snapshot_channel();
            object.bind_network_port_snapshot_internal(Some(Arc::new(reader)));
            ports.push(BoundPort {
                publisher,
                baseline,
            });
        }

        Self {
            ports,
            bbmd: observation.bbmd(),
            task: None,
        }
    }

    fn empty() -> Self {
        Self {
            ports: Vec::new(),
            bbmd: None,
            task: None,
        }
    }

    /// Publish the actual post-bind address and launch BBMD refreshes. This is
    /// called only after `NetworkLayer::start`, when an ephemeral UDP port has
    /// been resolved into the transport's local MAC.
    pub(super) async fn activate(&mut self, local_mac: &[u8], max_apdu_length: u32) {
        if self.ports.is_empty() || local_mac.len() != 6 {
            return;
        }
        let mut live_ports = with_bip_identity(&self.ports, local_mac, max_apdu_length);
        if let Some(control) = &self.bbmd {
            match control.snapshot().await {
                Ok(bbmd) => publish_bbmd(&live_ports, &bbmd),
                Err(_) => clear(&live_ports),
            }
            let control = control.clone();
            let refresh_ports = live_ports.clone();
            self.task = Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(PUBLISH_INTERVAL);
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    match control.snapshot().await {
                        Ok(snapshot) => publish_bbmd(&refresh_ports, &snapshot),
                        Err(_) => clear(&refresh_ports),
                    }
                }
            }));
        } else {
            for port in &mut live_ports {
                port.baseline.bacnet_ip_mode = IPMode::NORMAL.to_raw();
                port.publisher.publish(port.baseline.clone());
            }
        }
        self.ports = live_ports;
    }

    /// Stop refresh work and remove transport observations so reads use the
    /// NetworkPort's object-owned configuration again.
    pub(super) async fn stop(&mut self) {
        if let Some(task) = self.task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.task = None;
        clear(&self.ports);
    }
}

impl Drop for NetworkPortLiveController {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        clear(&self.ports);
    }
}

fn with_bip_identity(
    ports: &[BoundPort],
    local_mac: &[u8],
    max_apdu_length: u32,
) -> Vec<BoundPort> {
    ports
        .iter()
        .cloned()
        .map(|mut port| {
            port.baseline.mac_address = local_mac.to_vec();
            port.baseline.ip_address = local_mac[..4].to_vec();
            port.baseline.ip_udp_port = u16::from_be_bytes([local_mac[4], local_mac[5]]);
            port.baseline.max_apdu_length_accepted = max_apdu_length;
            port
        })
        .collect()
}

fn publish_bbmd(ports: &[BoundPort], bbmd: &BbmdSnapshot) {
    for port in ports {
        let mut snapshot = port.baseline.clone();
        snapshot.bacnet_ip_mode = IPMode::BBMD.to_raw();
        snapshot.bbmd_accept_fd_registrations = bbmd.accept_foreign_devices;
        snapshot.bbmd_broadcast_distribution_table =
            bbmd.bdt.iter().map(encoded_bdt_entry).collect();
        snapshot.bbmd_foreign_device_table = bbmd.fdt.iter().map(encoded_fdt_entry).collect();
        port.publisher.publish(snapshot);
    }
}

fn clear(ports: &[BoundPort]) {
    for port in ports {
        port.publisher.clear();
    }
}

fn encoded_bdt_entry(entry: &BdtEntry) -> PropertyValue {
    let mut bytes = Vec::with_capacity(10);
    bytes.extend_from_slice(&entry.ip);
    bytes.extend_from_slice(&entry.port.to_be_bytes());
    bytes.extend_from_slice(&entry.broadcast_mask);
    PropertyValue::OctetString(bytes)
}

fn encoded_fdt_entry(entry: &FdtEntryWire) -> PropertyValue {
    let mut bytes = Vec::with_capacity(10);
    bytes.extend_from_slice(&entry.ip);
    bytes.extend_from_slice(&entry.port.to_be_bytes());
    bytes.extend_from_slice(&entry.ttl.to_be_bytes());
    bytes.extend_from_slice(&entry.seconds_remaining.to_be_bytes());
    PropertyValue::OctetString(bytes)
}

fn baseline_snapshot(
    object: &dyn bacnet_objects::traits::BACnetObject,
) -> Option<NetworkPortLiveSnapshot> {
    Some(NetworkPortLiveSnapshot {
        network_type: enumerated(object, PropertyIdentifier::NETWORK_TYPE)?,
        network_number: unsigned(object, PropertyIdentifier::NETWORK_NUMBER)?,
        mac_address: octets(object, PropertyIdentifier::MAC_ADDRESS)?,
        max_apdu_length_accepted: unsigned(object, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)?,
        link_speed: real(object, PropertyIdentifier::LINK_SPEED)?,
        ip_address: octets(object, PropertyIdentifier::IP_ADDRESS)?,
        ip_default_gateway: octets(object, PropertyIdentifier::IP_DEFAULT_GATEWAY)?,
        ip_subnet_mask: octets(object, PropertyIdentifier::IP_SUBNET_MASK)?,
        ip_udp_port: u16::try_from(unsigned(object, PropertyIdentifier::BACNET_IP_UDP_PORT)?)
            .ok()?,
        bacnet_ip_mode: IPMode::NORMAL.to_raw(),
        bbmd_accept_fd_registrations: false,
        bbmd_broadcast_distribution_table: Vec::new(),
        bbmd_foreign_device_table: Vec::new(),
    })
}

fn unsigned(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
) -> Option<u32> {
    match object.read_property(property, None).ok()? {
        PropertyValue::Unsigned(value) => u32::try_from(value).ok(),
        _ => None,
    }
}

fn enumerated(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
) -> Option<u32> {
    match object.read_property(property, None).ok()? {
        PropertyValue::Enumerated(value) => Some(value),
        _ => None,
    }
}

fn octets(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
) -> Option<Vec<u8>> {
    match object.read_property(property, None).ok()? {
        PropertyValue::OctetString(value) => Some(value),
        _ => None,
    }
}

fn real(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
) -> Option<f32> {
    match object.read_property(property, None).ok()? {
        PropertyValue::Real(value) => Some(value),
        _ => None,
    }
}
