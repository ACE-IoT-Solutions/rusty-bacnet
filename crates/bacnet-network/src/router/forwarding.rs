use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::DataAttribute;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::MacAddr;
use bytes::{BufMut, BytesMut};
use tokio::sync::mpsc;
use tracing::warn;

use super::SendRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ForwardOutcome {
    Queued,
    HopExhausted,
    EncodeFailed,
    InvalidRoute,
    OutputFull { port: usize },
    OutputClosed { port: usize },
}

/// Build the source NpduAddress for a forwarded message.
fn build_source(npdu: &Npdu, source_network: u16, source_mac: &[u8]) -> NpduAddress {
    npdu.source.clone().unwrap_or(NpduAddress {
        network: source_network,
        mac_address: MacAddr::from_slice(source_mac),
    })
}

/// Forward a message to a specific destination via the route entry.
pub(super) fn forward_unicast(
    send_txs: &[mpsc::Sender<SendRequest>],
    route: &crate::router_table::RouteEntry,
    source_network: u16,
    source_mac: &[u8],
    npdu: Npdu,
    _source_port_idx: usize,
    data_attributes: &[DataAttribute],
) -> ForwardOutcome {
    if npdu.hop_count == 0 {
        warn!("Discarding NPDU with hop_count=0");
        return ForwardOutcome::HopExhausted;
    }

    let payload_len = npdu.payload.len();
    let source = build_source(&npdu, source_network, source_mac);
    let dest_mac;
    let forwarded_dest;
    let forwarded_hop_count;

    if route.directly_connected {
        // Directly connected: strip DNET/DADR/Hop Count from NPCI, send to DADR.
        dest_mac = npdu
            .destination
            .as_ref()
            .map(|d| d.mac_address.clone())
            .unwrap_or_default();
        forwarded_dest = None;
        forwarded_hop_count = 0; // not used without destination
    } else {
        dest_mac = route.next_hop_mac.clone();
        forwarded_dest = npdu.destination;
        forwarded_hop_count = npdu.hop_count - 1;
    };

    let forwarded = Npdu {
        is_network_message: npdu.is_network_message,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: forwarded_dest,
        source: Some(source),
        hop_count: forwarded_hop_count,
        message_type: None,
        vendor_id: None,
        payload: npdu.payload,
    };

    let mut buf = BytesMut::with_capacity(32 + payload_len);
    if let Err(e) = encode_npdu(&mut buf, &forwarded) {
        warn!("Failed to encode forwarded NPDU: {e}");
        return ForwardOutcome::EncodeFailed;
    }

    if route.port_index >= send_txs.len() {
        warn!(
            port = route.port_index,
            "Route references invalid port index"
        );
        return ForwardOutcome::InvalidRoute;
    }
    if dest_mac.is_empty() {
        match send_txs[route.port_index].try_send(SendRequest::Broadcast {
            npdu: buf.freeze(),
            data_attributes: data_attributes.to_vec(),
            count_forward: true,
        }) {
            Ok(()) => ForwardOutcome::Queued,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Router dropped message: output channel full");
                ForwardOutcome::OutputFull {
                    port: route.port_index,
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => ForwardOutcome::OutputClosed {
                port: route.port_index,
            },
        }
    } else {
        match send_txs[route.port_index].try_send(SendRequest::Unicast {
            npdu: buf.freeze(),
            mac: dest_mac,
            data_attributes: data_attributes.to_vec(),
            count_forward: true,
        }) {
            Ok(()) => ForwardOutcome::Queued,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Router dropped message: output channel full");
                ForwardOutcome::OutputFull {
                    port: route.port_index,
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => ForwardOutcome::OutputClosed {
                port: route.port_index,
            },
        }
    }
}

/// Forward a global broadcast to all ports except the source port.
pub(super) fn forward_broadcast(
    send_txs: &[mpsc::Sender<SendRequest>],
    source_port: usize,
    source_network: u16,
    source_mac: &[u8],
    npdu: &Npdu,
    data_attributes: &[DataAttribute],
) -> Vec<ForwardOutcome> {
    if npdu.hop_count == 0 {
        warn!("Discarding NPDU with hop_count=0");
        return vec![ForwardOutcome::HopExhausted];
    }

    let forwarded = Npdu {
        is_network_message: npdu.is_network_message,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
        destination: npdu.destination.clone(),
        source: Some(build_source(npdu, source_network, source_mac)),
        hop_count: npdu.hop_count - 1,
        message_type: npdu.message_type,
        vendor_id: npdu.vendor_id,
        payload: npdu.payload.clone(),
    };

    let mut buf = BytesMut::with_capacity(32 + npdu.payload.len());
    if let Err(e) = encode_npdu(&mut buf, &forwarded) {
        warn!("Failed to encode forwarded broadcast NPDU: {e}");
        return vec![ForwardOutcome::EncodeFailed];
    }

    let encoded = buf.freeze();
    let mut outcomes = Vec::with_capacity(send_txs.len().saturating_sub(1));
    for (idx, tx) in send_txs.iter().enumerate() {
        if idx == source_port {
            continue;
        }
        outcomes.push(
            match tx.try_send(SendRequest::Broadcast {
                npdu: encoded.clone(),
                data_attributes: data_attributes.to_vec(),
                count_forward: true,
            }) {
                Ok(()) => ForwardOutcome::Queued,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!("Router dropped broadcast: output channel full");
                    ForwardOutcome::OutputFull { port: idx }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    ForwardOutcome::OutputClosed { port: idx }
                }
            },
        );
    }
    outcomes
}

/// Send a Reject-Message-To-Network.
pub(super) fn send_reject(
    send_tx: &mpsc::Sender<SendRequest>,
    source_mac: &[u8],
    rejected_network: u16,
    reason: RejectMessageReason,
) {
    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(reason.to_raw());
    payload.put_u16(rejected_network);

    let reject = Npdu {
        is_network_message: true,
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
        payload: payload.freeze(),
        ..Npdu::default()
    };

    let mut buf = BytesMut::with_capacity(8);
    if let Err(e) = encode_npdu(&mut buf, &reject) {
        warn!("Failed to encode Reject-Message NPDU: {e}");
        return;
    }

    if let Err(e) = send_tx.try_send(SendRequest::unicast(
        buf.freeze(),
        MacAddr::from_slice(source_mac),
    )) {
        warn!(%e, "Router dropped reject message: output channel full");
    }
}
