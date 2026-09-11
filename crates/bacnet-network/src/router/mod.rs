//! BACnet half-router — forwards APDUs between BACnet networks.
//!
//! Per ASHRAE 135-2020 Clause 6.4, a BACnet router connects two or more
//! BACnet networks. It forwards messages between them by manipulating
//! the NPDU source/destination fields and decrementing the hop count.
//!
//! This implementation supports:
//! - Forwarding APDUs between directly-connected networks
//! - Who-Is-Router-To-Network / I-Am-Router-To-Network messages
//! - Reject-Message-To-Network for unknown routes
//! - Learned routes from I-Am-Router-To-Network announcements

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::layer::{is_group_delivery, ReceivedApdu};
use crate::router_table::{ReachabilityStatus, RouterTable};

mod control_messages;
mod forwarding;

use control_messages::handle_network_message;
use forwarding::{forward_broadcast, forward_unicast, send_reject, ForwardOutcome};

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

#[derive(Default)]
struct PortCounterState {
    forwarded_unicast: AtomicU64,
    forwarded_broadcast: AtomicU64,
    decode_drops: AtomicU64,
    encode_drops: AtomicU64,
    hop_drops: AtomicU64,
    busy_drops: AtomicU64,
    no_route_drops: AtomicU64,
    output_full_drops: AtomicU64,
    send_errors: AtomicU64,
    shutdown_drops: AtomicU64,
}

/// Monotonic forwarding counters for one configured router port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterPortCounters {
    /// Zero-based position in the original router configuration.
    pub config_index: usize,
    /// Configured BACnet network number.
    pub network_number: u16,
    /// Transport implementation used by this router instance.
    pub transport_kind: String,
    /// Local transport MAC after startup, formatted as hexadecimal octets.
    pub identity: String,
    /// Data NPDUs successfully sent as unicasts by this port.
    pub forwarded_unicast: u64,
    /// Data NPDU copies successfully sent as broadcasts by this port.
    pub forwarded_broadcast: u64,
    /// Ingress NPDUs discarded because NPDU decoding failed.
    pub decode_drops: u64,
    /// Ingress NPDUs discarded because forwarding encoding failed.
    pub encode_drops: u64,
    /// Ingress NPDUs discarded because their hop count was exhausted.
    pub hop_drops: u64,
    /// Ingress NPDUs discarded because no usable route existed.
    pub no_route_drops: u64,
    /// Ingress NPDUs discarded because the selected route was busy.
    pub busy_drops: u64,
    /// Data forwarding attempts discarded because the output queue was full.
    pub output_full_drops: u64,
    /// Data forwarding attempts that reached a transport but failed to send.
    pub send_errors: u64,
    /// Accepted data forwarding attempts cancelled during router shutdown.
    pub shutdown_drops: u64,
}

#[derive(Clone)]
struct PortCounterEntry {
    config_index: usize,
    network_number: u16,
    transport_kind: String,
    identity: String,
    state: Arc<PortCounterState>,
}

impl PortCounterEntry {
    fn snapshot(&self) -> RouterPortCounters {
        RouterPortCounters {
            config_index: self.config_index,
            network_number: self.network_number,
            transport_kind: self.transport_kind.clone(),
            identity: self.identity.clone(),
            forwarded_unicast: self.state.forwarded_unicast.load(Ordering::Relaxed),
            forwarded_broadcast: self.state.forwarded_broadcast.load(Ordering::Relaxed),
            decode_drops: self.state.decode_drops.load(Ordering::Relaxed),
            encode_drops: self.state.encode_drops.load(Ordering::Relaxed),
            hop_drops: self.state.hop_drops.load(Ordering::Relaxed),
            no_route_drops: self.state.no_route_drops.load(Ordering::Relaxed),
            busy_drops: self.state.busy_drops.load(Ordering::Relaxed),
            output_full_drops: self.state.output_full_drops.load(Ordering::Relaxed),
            send_errors: self.state.send_errors.load(Ordering::Relaxed),
            shutdown_drops: self.state.shutdown_drops.load(Ordering::Relaxed),
        }
    }
}

fn record_forward_outcome(
    counters: &[PortCounterEntry],
    ingress_port: usize,
    outcome: ForwardOutcome,
) {
    let state = match outcome {
        ForwardOutcome::Queued => return,
        ForwardOutcome::OutputFull { port } | ForwardOutcome::OutputClosed { port } => {
            counters.get(port).map(|entry| &entry.state)
        }
        _ => counters.get(ingress_port).map(|entry| &entry.state),
    };
    let Some(state) = state else { return };
    match outcome {
        ForwardOutcome::Queued => {}
        ForwardOutcome::HopExhausted => saturating_increment(&state.hop_drops),
        ForwardOutcome::EncodeFailed => saturating_increment(&state.encode_drops),
        ForwardOutcome::InvalidRoute => saturating_increment(&state.no_route_drops),
        ForwardOutcome::OutputFull { .. } => saturating_increment(&state.output_full_drops),
        ForwardOutcome::OutputClosed { .. } => saturating_increment(&state.shutdown_drops),
    }
}

/// A send request to be forwarded on a port.
#[derive(Debug)]
enum SendRequest {
    Unicast {
        npdu: Bytes,
        mac: MacAddr,
        data_attributes: Vec<DataAttribute>,
        count_forward: bool,
    },
    Broadcast {
        npdu: Bytes,
        data_attributes: Vec<DataAttribute>,
        count_forward: bool,
    },
}

impl SendRequest {
    fn unicast(npdu: Bytes, mac: MacAddr) -> Self {
        Self::Unicast {
            npdu,
            mac,
            data_attributes: Vec::new(),
            count_forward: false,
        }
    }

    fn broadcast(npdu: Bytes) -> Self {
        Self::Broadcast {
            npdu,
            data_attributes: Vec::new(),
            count_forward: false,
        }
    }
}

/// A router port: a transport bound to a specific BACnet network number.
pub struct RouterPort<T: TransportPort> {
    /// The transport for this port.
    pub transport: T,
    /// The network number assigned to this port.
    pub network_number: u16,
}

/// BACnet router connecting multiple networks.
///
/// The router holds multiple ports, each bound to a different BACnet network.
/// When an NPDU arrives on one port with a destination network that maps to
/// another port, the router forwards the message.
pub struct BACnetRouter {
    /// Shared routing table.
    table: Arc<Mutex<RouterTable>>,
    /// Dispatch tasks (one per port).
    dispatch_tasks: Vec<JoinHandle<()>>,
    /// Sender tasks (one per port, owns the transport for outgoing messages).
    sender_tasks: Vec<JoinHandle<()>>,
    /// Background task that purges stale learned routes.
    aging_task: Option<JoinHandle<()>>,
    /// Stable metadata and monotonic state for configured ports.
    counters: Vec<PortCounterEntry>,
}

impl BACnetRouter {
    /// Create and start a router from a list of ports.
    ///
    /// Returns the router and a receiver for APDUs destined to local
    /// applications (messages without remote destination or where this
    /// router is the final hop).
    pub async fn start<T: TransportPort + 'static>(
        mut ports: Vec<RouterPort<T>>,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>), Error> {
        let mut table = RouterTable::new();

        // Reject duplicate network numbers
        {
            let mut seen = std::collections::HashSet::new();
            for port in &ports {
                if !seen.insert(port.network_number) {
                    return Err(Error::Encoding(format!(
                        "Duplicate network number {} in router ports",
                        port.network_number
                    )));
                }
            }
        }

        // Register directly-connected networks
        for (idx, port) in ports.iter().enumerate() {
            table.add_direct(port.network_number, idx);
        }

        let table = Arc::new(Mutex::new(table));
        let (local_tx, local_rx) = mpsc::channel(256);

        // Start each transport, set up send channels
        let mut port_receivers = Vec::new();
        let mut send_txs: Vec<mpsc::Sender<SendRequest>> = Vec::new();
        let mut sender_tasks = Vec::new();
        let mut port_networks = Vec::new();
        let mut port_local_macs = Vec::new();

        let mut counters: Vec<PortCounterEntry> = ports
            .iter()
            .enumerate()
            .map(|(config_index, port)| PortCounterEntry {
                config_index,
                network_number: port.network_number,
                transport_kind: std::any::type_name::<T>().to_string(),
                identity: String::new(),
                state: Arc::new(PortCounterState::default()),
            })
            .collect();

        for (port_idx, port) in ports.iter_mut().enumerate() {
            let rx = port.transport.start().await?;
            port_receivers.push(rx);
            port_networks.push(port.network_number);
            port_local_macs.push(MacAddr::from_slice(port.transport.local_mac()));
            counters[port_idx].identity = port
                .transport
                .local_mac()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":");
        }

        // Move transports into sender tasks
        for (port_idx, port) in ports.into_iter().enumerate() {
            let (send_tx, mut send_rx) = mpsc::channel::<SendRequest>(256);
            send_txs.push(send_tx);

            let transport = port.transport;
            let counter = Arc::clone(&counters[port_idx].state);
            let task = tokio::spawn(async move {
                while let Some(req) = send_rx.recv().await {
                    match req {
                        SendRequest::Unicast {
                            npdu,
                            mac,
                            data_attributes,
                            count_forward,
                        } => {
                            if let Err(e) = transport
                                .send_unicast_with_data_attributes(&npdu, &mac, &data_attributes)
                                .await
                            {
                                if count_forward {
                                    saturating_increment(&counter.send_errors);
                                }
                                warn!(error = %e, "Router send_unicast failed");
                            } else if count_forward {
                                saturating_increment(&counter.forwarded_unicast);
                            }
                        }
                        SendRequest::Broadcast {
                            npdu,
                            data_attributes,
                            count_forward,
                        } => {
                            if let Err(e) = transport
                                .send_broadcast_with_data_attributes(&npdu, &data_attributes)
                                .await
                            {
                                if count_forward {
                                    saturating_increment(&counter.send_errors);
                                }
                                warn!(error = %e, "Router send_broadcast failed");
                            } else if count_forward {
                                saturating_increment(&counter.forwarded_broadcast);
                            }
                        }
                    }
                }
            });
            sender_tasks.push(task);
        }

        let send_txs = Arc::new(send_txs);

        // Announce I-Am-Router-To-Network on each port listing networks reachable via other ports.
        for (port_idx, tx) in send_txs.iter().enumerate() {
            let other_networks: Vec<u16> = port_networks
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != port_idx)
                .map(|(_, net)| *net)
                .collect();

            if other_networks.is_empty() {
                continue;
            }

            let mut payload = BytesMut::with_capacity(other_networks.len() * 2);
            for net in &other_networks {
                payload.put_u16(*net);
            }

            let payload_len = payload.len();
            let response = Npdu {
                is_network_message: true,
                message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
                payload: payload.freeze(),
                ..Npdu::default()
            };

            let mut buf = BytesMut::with_capacity(8 + payload_len);
            if let Err(e) = encode_npdu(&mut buf, &response) {
                warn!("Failed to encode I-Am-Router NPDU: {e}");
                continue;
            }

            if let Err(e) = tx.try_send(SendRequest::broadcast(buf.freeze())) {
                warn!(%e, "Router dropped I-Am-Router announcement: output channel full");
            }
        }

        let mut dispatch_tasks = Vec::new();

        for (port_idx, mut rx) in port_receivers.into_iter().enumerate() {
            let table = Arc::clone(&table);
            let local_tx = local_tx.clone();
            let send_txs = Arc::clone(&send_txs);
            let port_network = port_networks[port_idx];
            let local_mac = port_local_macs[port_idx].clone();
            let counter = Arc::clone(&counters[port_idx].state);
            let dispatch_counters = counters.clone();

            let task = tokio::spawn(async move {
                while let Some(received) = rx.recv().await {
                    match decode_npdu(received.npdu.clone()) {
                        Ok(npdu) => {
                            if npdu.is_network_message {
                                // Proprietary network messages (type >= 0x80) with DNET
                                // should be forwarded, not processed locally.
                                let is_proprietary =
                                    npdu.message_type.map(|t| t >= 0x80).unwrap_or(false);
                                let has_remote_dest = npdu
                                    .destination
                                    .as_ref()
                                    .is_some_and(|d| d.network != 0xFFFF);
                                if is_proprietary && has_remote_dest {
                                    // Fall through to normal DNET routing below
                                } else {
                                    handle_network_message(
                                        &table,
                                        &send_txs,
                                        port_idx,
                                        port_network,
                                        &received.source_mac,
                                        &npdu,
                                    )
                                    .await;
                                    continue;
                                }
                            }

                            if let Some(ref dest) = npdu.destination {
                                let dest_net = dest.network;

                                // Global broadcast — forward to all other ports
                                if dest_net == 0xFFFF {
                                    for outcome in forward_broadcast(
                                        &send_txs,
                                        port_idx,
                                        port_network,
                                        &received.source_mac,
                                        &npdu,
                                        &received.data_attributes,
                                    ) {
                                        record_forward_outcome(
                                            &dispatch_counters,
                                            port_idx,
                                            outcome,
                                        );
                                    }

                                    // Deliver locally as well
                                    let apdu = ReceivedApdu {
                                        apdu: npdu.payload,
                                        source_mac: received.source_mac,
                                        source_network: npdu.source,
                                        link_layer_group: received.link_layer_group,
                                        is_group: true,
                                        data_attributes: received.data_attributes,
                                        transport_meta: received.transport_meta,
                                        reply_tx: received.reply_tx,
                                    };
                                    let _ = local_tx.send(apdu).await;
                                    continue;
                                }

                                // Route lookup for destination network
                                let (route, reachability) = {
                                    let mut tbl = table.lock().await;
                                    let route = tbl.lookup(dest_net).cloned();
                                    let reachability = tbl.effective_reachability(dest_net);
                                    if route.is_some() {
                                        tbl.touch(dest_net);
                                    }
                                    (route, reachability)
                                };

                                if let Some(route) = route {
                                    // Check reachability before forwarding (spec 6.6.3.6)
                                    match reachability.unwrap_or(ReachabilityStatus::Reachable) {
                                        ReachabilityStatus::Busy => {
                                            saturating_increment(&counter.busy_drops);
                                            send_reject(
                                                &send_txs[port_idx],
                                                &received.source_mac,
                                                dest_net,
                                                RejectMessageReason::ROUTER_BUSY,
                                            );
                                            continue;
                                        }
                                        ReachabilityStatus::Unreachable => {
                                            saturating_increment(&counter.no_route_drops);
                                            send_reject(
                                                &send_txs[port_idx],
                                                &received.source_mac,
                                                dest_net,
                                                RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                                            );
                                            continue;
                                        }
                                        ReachabilityStatus::Reachable => {}
                                    }
                                    if route.port_index == port_idx && route.directly_connected {
                                        let dest_mac = npdu
                                            .destination
                                            .as_ref()
                                            .map(|d| &d.mac_address[..])
                                            .unwrap_or(&[]);
                                        if dest_mac == &local_mac[..] {
                                            // DADR matches our MAC: deliver locally
                                            let apdu = ReceivedApdu {
                                                apdu: npdu.payload,
                                                source_mac: received.source_mac,
                                                source_network: npdu.source,
                                                link_layer_group: received.link_layer_group,
                                                is_group: false,
                                                data_attributes: received.data_attributes,
                                                transport_meta: received.transport_meta,
                                                reply_tx: received.reply_tx,
                                            };
                                            let _ = local_tx.send(apdu).await;
                                        } else {
                                            // Remote broadcast to our network (DLEN=0):
                                            // deliver locally AND forward
                                            if dest_mac.is_empty() {
                                                let apdu = ReceivedApdu {
                                                    apdu: npdu.payload.clone(),
                                                    source_mac: received.source_mac.clone(),
                                                    source_network: npdu.source.clone(),
                                                    link_layer_group: received.link_layer_group,
                                                    is_group: true,
                                                    data_attributes: received
                                                        .data_attributes
                                                        .clone(),
                                                    transport_meta: received.transport_meta.clone(),
                                                    reply_tx: None,
                                                };
                                                let _ = local_tx.send(apdu).await;
                                            }
                                            let outcome = forward_unicast(
                                                &send_txs,
                                                &route,
                                                port_network,
                                                &received.source_mac,
                                                npdu,
                                                port_idx,
                                                &received.data_attributes,
                                            );
                                            record_forward_outcome(
                                                &dispatch_counters,
                                                port_idx,
                                                outcome,
                                            );
                                        }
                                    } else {
                                        let outcome = forward_unicast(
                                            &send_txs,
                                            &route,
                                            port_network,
                                            &received.source_mac,
                                            npdu,
                                            port_idx,
                                            &received.data_attributes,
                                        );
                                        record_forward_outcome(
                                            &dispatch_counters,
                                            port_idx,
                                            outcome,
                                        );
                                    }
                                } else {
                                    // Unknown network: send reject
                                    saturating_increment(&counter.no_route_drops);
                                    send_reject(
                                        &send_txs[port_idx],
                                        &received.source_mac,
                                        dest_net,
                                        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                                    );
                                }
                            } else {
                                let apdu = ReceivedApdu {
                                    apdu: npdu.payload,
                                    source_mac: received.source_mac,
                                    source_network: npdu.source,
                                    link_layer_group: received.link_layer_group,
                                    is_group: is_group_delivery(received.link_layer_group, None),
                                    data_attributes: received.data_attributes,
                                    transport_meta: received.transport_meta,
                                    reply_tx: received.reply_tx,
                                };
                                let _ = local_tx.send(apdu).await;
                            }
                        }
                        Err(e) => {
                            saturating_increment(&counter.decode_drops);
                            warn!(error = %e, port = port_idx, "Router decode failed");
                        }
                    }
                }
            });

            dispatch_tasks.push(task);
        }

        // Periodically purge stale learned routes.
        let aging_table = Arc::clone(&table);
        let aging_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            let max_age = Duration::from_secs(300); // 5 minutes
            loop {
                interval.tick().await;
                let mut tbl = aging_table.lock().await;
                let purged = tbl.purge_stale(max_age);
                tbl.clear_expired_busy();
                drop(tbl);
                for net in purged {
                    debug!(network = net, "Purged stale route");
                }
            }
        });

        Ok((
            Self {
                table,
                dispatch_tasks,
                sender_tasks,
                aging_task: Some(aging_task),
                counters,
            },
            local_rx,
        ))
    }

    /// Get a reference to the routing table.
    pub fn table(&self) -> &Arc<Mutex<RouterTable>> {
        &self.table
    }

    /// Return one stable snapshot per configured port, in configuration order.
    pub fn port_counters(&self) -> Vec<RouterPortCounters> {
        self.counters
            .iter()
            .map(PortCounterEntry::snapshot)
            .collect()
    }

    /// Stop the router.
    pub async fn stop(&mut self) {
        for task in self.dispatch_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        for task in self.sender_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.aging_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests;
