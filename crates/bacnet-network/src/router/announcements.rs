use std::time::Duration;

use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::port::{TransportHealth, TransportHealthState};
use bacnet_types::enums::NetworkMessageType;
use bacnet_types::error::Error;
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::warn;

use super::SendRequest;

/// Optional background behavior for a started BACnet router.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouterConfig {
    /// Re-advertise directly connected networks at this interval.
    ///
    /// `None`, the default, preserves the one-time startup announcement.
    /// A zero duration is rejected by [`super::BACnetRouter::start_with_config`].
    pub announce_interval: Option<Duration>,
}

pub(super) fn encode_for_ports(port_networks: &[u16]) -> Result<Vec<Option<Bytes>>, Error> {
    port_networks
        .iter()
        .enumerate()
        .map(|(port_idx, _)| {
            let other_networks = port_networks
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != port_idx)
                .map(|(_, network)| *network);
            let mut payload = BytesMut::with_capacity(port_networks.len().saturating_sub(1) * 2);
            for network in other_networks {
                payload.put_u16(network);
            }
            if payload.is_empty() {
                return Ok(None);
            }

            let payload_len = payload.len();
            let response = Npdu {
                is_network_message: true,
                message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
                payload: payload.freeze(),
                ..Npdu::default()
            };
            let mut encoded = BytesMut::with_capacity(8 + payload_len);
            encode_npdu(&mut encoded, &response)?;
            Ok(Some(encoded.freeze()))
        })
        .collect()
}

fn announce(tx: &mpsc::Sender<SendRequest>, npdu: &Option<Bytes>, trigger: &'static str) {
    let Some(npdu) = npdu else {
        return;
    };
    if let Err(error) = tx.try_send(SendRequest::broadcast(npdu.clone())) {
        warn!(%error, trigger, "Router dropped I-Am-Router announcement");
    }
}

pub(super) fn announce_all(
    send_txs: &[mpsc::Sender<SendRequest>],
    announcements: &[Option<Bytes>],
    trigger: &'static str,
) {
    for (tx, npdu) in send_txs.iter().zip(announcements) {
        announce(tx, npdu, trigger);
    }
}

pub(super) fn spawn_periodic(
    interval: Option<Duration>,
    send_txs: Vec<mpsc::Sender<SendRequest>>,
    announcements: Vec<Option<Bytes>>,
) -> Option<JoinHandle<()>> {
    let period = interval?;
    Some(tokio::spawn(async move {
        let start = tokio::time::Instant::now() + period;
        let mut ticker = tokio::time::interval_at(start, period);
        loop {
            ticker.tick().await;
            announce_all(&send_txs, &announcements, "periodic");
        }
    }))
}

pub(super) fn spawn_recovery(
    health_receivers: Vec<Option<watch::Receiver<TransportHealth>>>,
    send_txs: &[mpsc::Sender<SendRequest>],
    announcements: &[Option<Bytes>],
) -> Vec<JoinHandle<()>> {
    health_receivers
        .into_iter()
        .zip(send_txs)
        .zip(announcements)
        .filter_map(|((health, tx), npdu)| {
            let mut health = health?;
            let tx = tx.clone();
            let npdu = npdu.clone();
            Some(tokio::spawn(async move {
                let mut previous = health.borrow().state;
                while health.changed().await.is_ok() {
                    let current = health.borrow().state;
                    if current == TransportHealthState::Up && previous != TransportHealthState::Up {
                        announce(&tx, &npdu, "port-recovery");
                    }
                    previous = current;
                }
            }))
        })
        .collect()
}
