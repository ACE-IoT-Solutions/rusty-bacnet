use super::*;
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_types::enums::NetworkMessageType;
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROUTER_RESPONDERS: usize = 256;
const MAX_NETWORKS_PER_ROUTER: usize = 1_024;

/// One router responder and the complete set of networks it advertised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterInfo {
    /// Immediate transport source MAC, including the UDP port for BACnet/IP.
    pub source_mac: MacAddr,
    /// Routed NPDU source, when the announcement arrived through another router.
    pub source_network: Option<NpduAddress>,
    /// Sorted, unique reachable network numbers advertised by this source.
    pub networks: Vec<u16>,
}

type RouterKey = (Vec<u8>, Option<(u16, Vec<u8>)>);

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Return the latest incrementally collected router-discovery snapshot.
    ///
    /// A query clears the previous snapshot before transmitting. Each valid
    /// response updates it, so cancellation leaves a useful partial result.
    pub async fn router_snapshot(&self) -> Vec<RouterInfo> {
        self.router_snapshot.lock().await.clone()
    }

    /// Broadcast Who-Is-Router-To-Network and collect announcements for the
    /// supplied observation window.
    pub async fn who_is_router_to_network(
        &self,
        network: Option<u16>,
        observation_window: Duration,
    ) -> Result<Vec<RouterInfo>, Error> {
        if matches!(network, Some(0 | u16::MAX)) {
            return Err(Error::OutOfRange(
                "router query network must be in 1..=65534".into(),
            ));
        }

        let _query_guard = self.router_discovery_lock.lock().await;
        self.router_snapshot.lock().await.clear();
        let mut responses = self.network_control_tx.subscribe();

        let payload = network
            .map(|network| Bytes::copy_from_slice(&network.to_be_bytes()))
            .unwrap_or_default();
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw()),
            payload,
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &npdu)?;
        self.network.transport().send_broadcast(&encoded).await?;

        let deadline = tokio::time::Instant::now() + observation_window;
        let mut routers: BTreeMap<RouterKey, BTreeSet<u16>> = BTreeMap::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let message = match tokio::time::timeout(remaining, responses.recv()).await {
                Err(_) => break,
                Ok(Ok(message)) => message,
                Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                    return Err(Error::Encoding(format!(
                        "router discovery response channel lagged by {skipped} messages"
                    )));
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(Error::Encoding(
                        "router discovery response channel closed".into(),
                    ));
                }
            };

            if message.npdu.message_type
                != Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            {
                continue;
            }
            merge_router_announcement(&mut routers, &message)?;
            *self.router_snapshot.lock().await = router_infos(&routers);
        }

        Ok(router_infos(&routers))
    }
}

fn merge_router_announcement(
    routers: &mut BTreeMap<RouterKey, BTreeSet<u16>>,
    message: &ReceivedNetworkControl,
) -> Result<(), Error> {
    let payload = &message.npdu.payload;
    if payload.is_empty() || payload.len() % 2 != 0 {
        return Err(Error::decoding(
            0,
            "I-Am-Router-To-Network payload must contain one or more 16-bit networks",
        ));
    }

    let routed_source = message
        .npdu
        .source
        .as_ref()
        .map(|source| (source.network, source.mac_address.to_vec()));
    let key = (message.source_mac.to_vec(), routed_source);
    if !routers.contains_key(&key) && routers.len() >= MAX_ROUTER_RESPONDERS {
        return Err(Error::Encoding(format!(
            "router discovery exceeded {MAX_ROUTER_RESPONDERS} responders"
        )));
    }
    let advertised = routers.entry(key).or_default();
    for chunk in payload.chunks_exact(2) {
        let network = u16::from_be_bytes([chunk[0], chunk[1]]);
        if network == 0 || network == u16::MAX {
            return Err(Error::decoding(
                0,
                format!("I-Am-Router-To-Network advertised invalid network {network}"),
            ));
        }
        if !advertised.contains(&network) && advertised.len() >= MAX_NETWORKS_PER_ROUTER {
            return Err(Error::Encoding(format!(
                "router advertised more than {MAX_NETWORKS_PER_ROUTER} networks"
            )));
        }
        advertised.insert(network);
    }
    Ok(())
}

fn router_infos(routers: &BTreeMap<RouterKey, BTreeSet<u16>>) -> Vec<RouterInfo> {
    routers
        .iter()
        .map(|((source_mac, routed_source), networks)| RouterInfo {
            source_mac: MacAddr::from_vec(source_mac.clone()),
            source_network: routed_source.as_ref().map(|(network, mac)| NpduAddress {
                network: *network,
                mac_address: MacAddr::from_vec(mac.clone()),
            }),
            networks: networks.iter().copied().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::npdu::decode_npdu;
    use bacnet_transport::loopback::LoopbackTransport;

    fn control(
        source_mac: &[u8],
        source: Option<NpduAddress>,
        payload: &[u8],
    ) -> ReceivedNetworkControl {
        ReceivedNetworkControl {
            npdu: Npdu {
                is_network_message: true,
                message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
                source,
                payload: Bytes::copy_from_slice(payload),
                ..Npdu::default()
            },
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            transport_meta: None,
            ingress_sequence: 1,
        }
    }

    async fn send_router_announcement(transport: &LoopbackTransport, payload: &[u8]) {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()),
            payload: Bytes::copy_from_slice(payload),
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &npdu).unwrap();
        transport.send_broadcast(&encoded).await.unwrap();
    }

    #[test]
    fn merges_repeated_claims_and_preserves_routed_source() {
        let mut routers = BTreeMap::new();
        let source = Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[8]),
        });
        merge_router_announcement(
            &mut routers,
            &control(&[192, 0, 2, 10, 0xba, 0xc0], source.clone(), &[0, 2, 0, 1]),
        )
        .unwrap();
        merge_router_announcement(
            &mut routers,
            &control(&[192, 0, 2, 10, 0xba, 0xc0], source.clone(), &[0, 3, 0, 2]),
        )
        .unwrap();

        let infos = router_infos(&routers);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].networks, vec![1, 2, 3]);
        assert_eq!(infos[0].source_network, source);
    }

    #[test]
    fn malformed_and_reserved_networks_are_rejected() {
        let mut routers = BTreeMap::new();
        assert!(matches!(
            merge_router_announcement(&mut routers, &control(&[1], None, &[1])),
            Err(Error::Decoding { .. })
        ));
        assert!(matches!(
            merge_router_announcement(&mut routers, &control(&[1], None, &[0, 0])),
            Err(Error::Decoding { .. })
        ));
        assert!(matches!(
            merge_router_announcement(&mut routers, &control(&[1], None, &[0xff, 0xff])),
            Err(Error::Decoding { .. })
        ));
    }

    #[tokio::test]
    async fn discovery_merges_announcements_from_the_control_multiplexer() {
        let router_mac = MacAddr::from_slice(&[192, 0, 2, 10, 0xba, 0xc1]);
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(MacAddr::from_slice(&[0x01]), router_mac.clone());
        let mut router_rx = router_transport.start().await.unwrap();
        let router_task = tokio::spawn(async move {
            let request = decode_npdu(router_rx.recv().await.unwrap().npdu).unwrap();
            assert_eq!(
                request.message_type,
                Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(request.payload.as_ref(), &[0x07, 0xd1]);
            send_router_announcement(&router_transport, &[0x07, 0xd1, 0x03, 0xe8]).await;
            tokio::task::yield_now().await;
            send_router_announcement(&router_transport, &[0x03, 0xe8, 0x0b, 0xb8]).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        });

        let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap();
        let routers = client
            .who_is_router_to_network(Some(2001), Duration::from_millis(250))
            .await
            .unwrap();
        router_task.await.unwrap();
        assert_eq!(routers.len(), 1);
        assert_eq!(routers[0].source_mac, router_mac);
        assert_eq!(routers[0].networks, vec![1000, 2001, 3000]);
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_observation_retains_partial_snapshot() {
        let router_mac = MacAddr::from_slice(&[192, 0, 2, 12, 0xba, 0xc2]);
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(MacAddr::from_slice(&[0x01]), router_mac.clone());
        let mut router_rx = router_transport.start().await.unwrap();
        let router_task = tokio::spawn(async move {
            router_rx.recv().await.unwrap();
            send_router_announcement(&router_transport, &[0x0f, 0xa0]).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        });

        let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(500),
            client.who_is_router_to_network(None, Duration::from_secs(10)),
        )
        .await
        .is_err());
        router_task.await.unwrap();
        let snapshot = client.router_snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].source_mac, router_mac);
        assert_eq!(snapshot[0].networks, vec![4000]);
        client.stop().await.unwrap();
    }
}
