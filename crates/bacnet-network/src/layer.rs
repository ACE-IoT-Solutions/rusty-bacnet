//! NetworkLayer for local BACnet packet assembly and dispatch.
//!
//! The network layer wraps a transport and provides APDU-level send/receive
//! by handling NPDU encoding/decoding. This is a non-router implementation:
//! it does not forward messages between networks, but it can address remote
//! devices through local routers via NPDU destination fields (DNET/DADR).

use crate::observer::{decode_npdu_event, ApduDirection, ApduObserver};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::{DataAttribute, TransportMeta, TransportPort};
use bacnet_types::enums::NetworkPriority;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// A received APDU with source addressing information.
pub struct ReceivedApdu {
    /// Raw APDU bytes.
    pub apdu: Bytes,
    /// Source MAC address in transport-native format.
    pub source_mac: MacAddr,
    /// Source network address if the APDU was routed (NPDU had source field).
    pub source_network: Option<NpduAddress>,
    /// Whether the NPDU arrived through a data-link multicast or broadcast.
    ///
    /// This preserves raw data-link provenance independently of [`Self::is_group`].
    pub link_layer_group: bool,
    /// Whether the APDU's effective BACnet destination was multicast or broadcast.
    ///
    /// A specific DNET/DADR remains a unicast even when a router used a
    /// group data-link destination to reach that remote device.
    pub is_group: bool,
    /// Data-link attributes associated with the NPDU, if the transport supplied any.
    pub data_attributes: Vec<DataAttribute>,
    /// BACnet/IP framing and immediate UDP peer context, when applicable.
    pub transport_meta: Option<TransportMeta>,
    /// Optional reply channel for MS/TP DataExpectingReply flows.
    /// The application layer can send NPDU-wrapped reply bytes through this channel.
    pub reply_tx: Option<oneshot::Sender<Bytes>>,
}

/// A decoded network-layer control message received from a transport peer.
///
/// Non-router users opt in to this stream before [`NetworkLayer::start`].
/// Without that opt-in, network messages retain their historical discard/log
/// behavior.
#[derive(Debug, Clone)]
pub struct ReceivedNetworkControl {
    /// Decoded NPDU, including network-message type and typed-address fields.
    pub npdu: Npdu,
    /// Immediate transport peer that sent the message.
    pub source_mac: MacAddr,
    /// Whether the data-link delivery was multicast or broadcast.
    pub link_layer_group: bool,
    /// Data-link attributes supplied by the transport.
    pub data_attributes: Vec<DataAttribute>,
    /// BACnet/IP framing and immediate UDP peer context, when applicable.
    pub transport_meta: Option<TransportMeta>,
    /// Monotonic decoded-ingress sequence assigned before channel delivery.
    ///
    /// A consumer can compare this with [`NetworkLayer::network_control_ingress_sequence`]
    /// when activating state so controls already queued at that point cannot
    /// be mistaken for feedback about the new state.
    pub ingress_sequence: u64,
}

impl Clone for ReceivedApdu {
    fn clone(&self) -> Self {
        Self {
            apdu: self.apdu.clone(),
            source_mac: self.source_mac.clone(),
            source_network: self.source_network.clone(),
            link_layer_group: self.link_layer_group,
            is_group: self.is_group,
            data_attributes: self.data_attributes.clone(),
            transport_meta: self.transport_meta.clone(),
            reply_tx: None,
        }
    }
}

impl std::fmt::Debug for ReceivedApdu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedApdu")
            .field("apdu", &self.apdu)
            .field("source_mac", &self.source_mac)
            .field("source_network", &self.source_network)
            .field("link_layer_group", &self.link_layer_group)
            .field("is_group", &self.is_group)
            .field("data_attributes", &self.data_attributes)
            .field("transport_meta", &self.transport_meta)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(...)"))
            .finish()
    }
}

pub(crate) fn is_group_delivery(link_layer_group: bool, destination: Option<&NpduAddress>) -> bool {
    match destination {
        None => link_layer_group,
        Some(destination) => destination.network == 0xFFFF || destination.mac_address.is_empty(),
    }
}

/// Non-router BACnet network layer.
///
/// Wraps a [`TransportPort`] and provides APDU-level send/receive by handling
/// NPDU framing. This layer does not act as a router (it does not forward
/// messages between networks), but it can send to remote devices through
/// local routers using NPDU destination addressing.
pub struct NetworkLayer<T: TransportPort> {
    transport: T,
    dispatch_task: Option<JoinHandle<()>>,
    network_control_tx: Option<mpsc::Sender<ReceivedNetworkControl>>,
    network_control_ingress_sequence: Arc<AtomicU64>,
    observer: Option<ApduObserver>,
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Create a new network layer wrapping the given transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            dispatch_task: None,
            network_control_tx: None,
            network_control_ingress_sequence: Arc::new(AtomicU64::new(0)),
            observer: None,
        }
    }

    /// Create a network layer with bounded, passive APDU diagnostics enabled.
    pub fn with_observer(transport: T, observer: ApduObserver) -> Self {
        let mut layer = Self::new(transport);
        layer.observer = Some(observer);
        layer
    }

    /// Enable the one-consumer decoded network-control stream.
    ///
    /// This must be called before [`Self::start`] and at most once. Dropping
    /// the returned receiver disables control delivery without affecting APDU
    /// ingress.
    pub fn enable_network_control_receiver(
        &mut self,
    ) -> Result<mpsc::Receiver<ReceivedNetworkControl>, Error> {
        if self.dispatch_task.is_some() {
            return Err(Error::Encoding(
                "network-control receiver must be enabled before NetworkLayer::start".into(),
            ));
        }
        if self.network_control_tx.is_some() {
            return Err(Error::Encoding(
                "network-control receiver is already enabled".into(),
            ));
        }
        let (tx, rx) = mpsc::channel(256);
        self.network_control_tx = Some(tx);
        Ok(rx)
    }

    /// Start the network layer. Returns a receiver for incoming APDUs.
    ///
    /// This starts the underlying transport and spawns a dispatch task that
    /// decodes incoming NPDUs and extracts APDUs.
    pub async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedApdu>, Error> {
        let mut npdu_rx = self.transport.start().await?;
        let mut network_control_tx = self.network_control_tx.take();
        let network_control_ingress_sequence = Arc::clone(&self.network_control_ingress_sequence);
        let observer = self.observer.clone();

        let (apdu_tx, apdu_rx) = mpsc::channel(256);

        let dispatch_task = tokio::spawn(async move {
            while let Some(received) = npdu_rx.recv().await {
                if let Some(observer) = observer.as_ref() {
                    // Network messages are not APDUs. Malformed NPDUs remain
                    // observable, but observer decoding never participates in
                    // the admission decision below.
                    let is_apdu_or_malformed = decode_npdu(received.npdu.clone())
                        .map(|npdu| !npdu.is_network_message)
                        .unwrap_or(true);
                    if is_apdu_or_malformed {
                        observer.publish(decode_npdu_event(
                            ApduDirection::Inbound,
                            received.source_mac.clone(),
                            None,
                            received.npdu.clone(),
                        ));
                    }
                }
                match decode_npdu(received.npdu.clone()) {
                    Ok(npdu) => {
                        if npdu.is_network_message {
                            if let Some(tx) = network_control_tx.as_ref() {
                                let ingress_sequence = next_ingress_sequence(
                                    network_control_ingress_sequence.as_ref(),
                                );
                                let control = ReceivedNetworkControl {
                                    npdu,
                                    source_mac: received.source_mac,
                                    link_layer_group: received.link_layer_group,
                                    data_attributes: received.data_attributes,
                                    transport_meta: received.transport_meta,
                                    ingress_sequence,
                                };
                                if tx.send(control).await.is_err() {
                                    network_control_tx = None;
                                    debug!("Network-control receiver closed; resuming discard behavior");
                                }
                            } else {
                                debug!(
                                    message_type = npdu.message_type,
                                    "Ignoring network layer message (non-router mode)"
                                );
                            }
                            continue;
                        }

                        // Non-routing node: discard messages with a specific DNET.
                        if let Some(ref dest) = npdu.destination {
                            if dest.network != 0xFFFF {
                                debug!(
                                    dnet = dest.network,
                                    "Discarding routed message (non-router)"
                                );
                                continue;
                            }
                        }

                        let source_network = npdu.source.clone();
                        let is_group =
                            is_group_delivery(received.link_layer_group, npdu.destination.as_ref());

                        let apdu = ReceivedApdu {
                            apdu: npdu.payload,
                            source_mac: received.source_mac,
                            source_network,
                            link_layer_group: received.link_layer_group,
                            is_group,
                            data_attributes: received.data_attributes,
                            transport_meta: received.transport_meta,
                            reply_tx: received.reply_tx,
                        };

                        if apdu_tx.send(apdu).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode NPDU");
                    }
                }
            }
        });

        self.dispatch_task = Some(dispatch_task);

        Ok(apdu_rx)
    }

    /// Send an APDU to a specific local destination by MAC address.
    pub async fn send_apdu(
        &self,
        apdu: &[u8],
        destination_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.send_apdu_with_data_attributes(apdu, destination_mac, expecting_reply, priority, &[])
            .await
    }

    /// Send an APDU with data attributes to a specific local destination.
    pub async fn send_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        destination_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: None,
            source: None,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(2 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;

        self.observe_outbound(&buf, destination_mac);

        self.transport
            .send_unicast_with_data_attributes(&buf, destination_mac, data_attributes)
            .await
    }

    /// Broadcast an APDU on the local network.
    pub async fn broadcast_apdu(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.broadcast_apdu_with_data_attributes(apdu, expecting_reply, priority, &[])
            .await
    }

    /// Broadcast an APDU with data attributes on the local network.
    pub async fn broadcast_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: None,
            source: None,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(2 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;

        self.observe_outbound(&buf, &[]);

        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
    }

    /// Broadcast an APDU globally (DNET=0xFFFF, hop_count=255).
    ///
    /// Unlike `broadcast_apdu()` which only reaches the local subnet, this
    /// sets DNET=0xFFFF so routers will forward to all reachable networks.
    pub async fn broadcast_global_apdu(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.broadcast_global_apdu_with_data_attributes(apdu, expecting_reply, priority, &[])
            .await
    }

    /// Broadcast an APDU globally with data attributes (DNET=0xFFFF, hop_count=255).
    pub async fn broadcast_global_apdu_with_data_attributes(
        &self,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;
        self.observe_outbound(&buf, &[]);
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
    }

    /// Broadcast an APDU to a specific remote network via routers.
    ///
    /// Like `broadcast_global_apdu()` but targets a single network number
    /// instead of all networks (DNET=0xFFFF).
    pub async fn broadcast_to_network(
        &self,
        apdu: &[u8],
        dest_network: u16,
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.broadcast_to_network_with_data_attributes(
            apdu,
            dest_network,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Broadcast an APDU with data attributes to a specific remote network via routers.
    pub async fn broadcast_to_network_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        if dest_network == 0xFFFF {
            return Err(Error::Encoding(
                "dest_network 0xFFFF is reserved for global broadcasts; use broadcast_global_apdu instead".into(),
            ));
        }
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: dest_network,
                mac_address: MacAddr::new(),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };

        let mut buf = BytesMut::with_capacity(8 + apdu.len());
        encode_npdu(&mut buf, &npdu)?;
        self.observe_outbound(&buf, &[]);
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
    }

    /// Send an APDU to a remote device through a local router.
    ///
    /// The NPDU is sent via unicast to `router_mac` (the next-hop router on
    /// the local network), but the NPDU header addresses the final destination
    /// with `dest_network` / `dest_mac`.
    pub async fn send_apdu_routed(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        router_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.send_apdu_routed_with_data_attributes(
            apdu,
            dest_network,
            dest_mac,
            router_mac,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Send an APDU with data attributes to a remote device through a local router.
    pub async fn send_apdu_routed_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        router_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let buf =
            Self::encode_routed_npdu_buf(apdu, dest_network, dest_mac, expecting_reply, priority)?;
        self.observe_outbound(&buf, router_mac);
        self.transport
            .send_unicast_with_data_attributes(&buf, router_mac, data_attributes)
            .await
    }

    /// Send a routed APDU with a broadcast link DA, for when the next-hop
    /// router's MAC is unknown.
    ///
    /// Clause 6.5.3: the data link DA "shall be the MAC address of the BACnet
    /// router corresponding to the DNET parameter or the appropriate
    /// broadcast DA if the address of the router is initially unknown". The
    /// NPDU still addresses one device via DNET/DADR, which is why Clause
    /// 6.3's broadcast restriction does not bite: "a MAC layer multicast or
    /// broadcast address may be used for other PDU types when the network
    /// layer address restricts the destination to a single device".
    pub async fn send_apdu_routed_via_local_broadcast(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        self.send_apdu_routed_via_local_broadcast_with_data_attributes(
            apdu,
            dest_network,
            dest_mac,
            expecting_reply,
            priority,
            &[],
        )
        .await
    }

    /// Send a routed APDU with data attributes and a broadcast link DA.
    pub async fn send_apdu_routed_via_local_broadcast_with_data_attributes(
        &self,
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let buf =
            Self::encode_routed_npdu_buf(apdu, dest_network, dest_mac, expecting_reply, priority)?;
        self.observe_outbound(&buf, &[]);
        self.transport
            .send_broadcast_with_data_attributes(&buf, data_attributes)
            .await
    }

    /// Access the underlying transport.
    ///
    /// Useful for transport-specific operations like BBMD registration
    /// after the network layer has been started.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Get the transport's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        self.transport.local_mac()
    }

    /// Sequence assigned to the most recently decoded network-control ingress.
    ///
    /// The value is updated before the control is queued for its opt-in
    /// consumer. At counter exhaustion it remains at `u64::MAX`, which makes
    /// sequence-based consumers fail closed rather than accepting an alias.
    pub fn network_control_ingress_sequence(&self) -> u64 {
        self.network_control_ingress_sequence.load(Ordering::SeqCst)
    }

    /// Encode an APDU into an NPDU whose destination is `dest_network` /
    /// `dest_mac`, ready for whichever link send the caller chooses.
    fn encode_routed_npdu_buf(
        apdu: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<BytesMut, Error> {
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply,
            priority,
            destination: Some(NpduAddress {
                network: dest_network,
                mac_address: MacAddr::from_slice(dest_mac),
            }),
            source: None,
            hop_count: 255,
            payload: Bytes::copy_from_slice(apdu),
            ..Npdu::default()
        };
        let mut buf = BytesMut::with_capacity(8 + dest_mac.len() + apdu.len());
        encode_npdu(&mut buf, &npdu)?;
        Ok(buf)
    }

    /// Stop the network layer and underlying transport.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.abort_dispatch_task() {
            let _ = task.await;
        }
        let result = self.transport.stop().await;
        if let Some(observer) = self.observer.as_ref() {
            observer.close();
        }
        result
    }

    /// Record an already encoded outbound NPDU, such as an MS/TP reply.
    pub fn observe_outbound_npdu(&self, npdu: Bytes, immediate_peer: &[u8]) {
        if let Some(observer) = self.observer.as_ref() {
            observer.publish(decode_npdu_event(
                ApduDirection::Outbound,
                MacAddr::from_slice(immediate_peer),
                None,
                npdu,
            ));
        }
    }

    fn observe_outbound(&self, npdu: &[u8], immediate_peer: &[u8]) {
        self.observe_outbound_with(immediate_peer, || Bytes::copy_from_slice(npdu));
    }

    fn observe_outbound_with<F>(&self, immediate_peer: &[u8], make_npdu: F)
    where
        F: FnOnce() -> Bytes,
    {
        let Some(observer) = self.observer.as_ref() else {
            return;
        };
        observer.publish(decode_npdu_event(
            ApduDirection::Outbound,
            MacAddr::from_slice(immediate_peer),
            None,
            make_npdu(),
        ));
    }
}

fn next_ingress_sequence(sequence: &AtomicU64) -> u64 {
    let previous = sequence
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        })
        .unwrap_or(u64::MAX);
    previous.saturating_add(1)
}

impl<T: TransportPort> NetworkLayer<T> {
    fn abort_dispatch_task(&mut self) -> Option<JoinHandle<()>> {
        let task = self.dispatch_task.take()?;
        task.abort();
        Some(task)
    }
}

impl<T: TransportPort> Drop for NetworkLayer<T> {
    fn drop(&mut self) {
        let _ = self.abort_dispatch_task();
        self.transport.abort();
        if let Some(observer) = self.observer.as_ref() {
            observer.close();
        }
    }
}

#[cfg(test)]
#[path = "layer/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "layer_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "layer_sc_data_options_tests.rs"]
mod sc_data_options_tests;
