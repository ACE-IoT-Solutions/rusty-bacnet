use super::*;
use std::collections::{BTreeMap, BTreeSet};

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

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Return the latest incrementally collected router-discovery snapshot.
    ///
    /// The snapshot is cleared when a query starts and updated after each valid
    /// response, so cancellation leaves partial results available separately
    /// from protocol and transport failures.
    pub async fn router_snapshot(&self) -> Vec<RouterInfo> {
        self.router_snapshot.lock().await.clone()
    }

    /// Broadcast Who-Is-Router-To-Network and collect announcements for `observation_window`.
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

        self.router_snapshot.lock().await.clear();

        let mut responses = self.network.subscribe_network_messages();
        let payload = network
            .map(|network| network.to_be_bytes().to_vec())
            .unwrap_or_default();
        self.network
            .broadcast_network_message(
                bacnet_types::enums::NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw(),
                payload.as_slice(),
            )
            .await?;

        let deadline = tokio::time::Instant::now() + observation_window;
        let mut routers: BTreeMap<(Vec<u8>, Option<(u16, Vec<u8>)>), BTreeSet<u16>> =
            BTreeMap::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let message = match timeout(remaining, responses.recv()).await {
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

            if message.message_type
                != bacnet_types::enums::NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw()
            {
                continue;
            }
            if message.payload.is_empty() || message.payload.len() % 2 != 0 {
                return Err(Error::decoding(
                    0,
                    "I-Am-Router-To-Network payload must contain one or more 16-bit networks",
                ));
            }

            let routed_source = message
                .source_network
                .as_ref()
                .map(|source| (source.network, source.mac_address.to_vec()));
            let key = (message.source_mac.to_vec(), routed_source);
            let advertised = routers.entry(key).or_default();
            for chunk in message.payload.chunks_exact(2) {
                let advertised_network = u16::from_be_bytes([chunk[0], chunk[1]]);
                if advertised_network == 0 || advertised_network == u16::MAX {
                    return Err(Error::decoding(
                        0,
                        format!(
                            "I-Am-Router-To-Network advertised invalid network {advertised_network}"
                        ),
                    ));
                }
                advertised.insert(advertised_network);
            }
            *self.router_snapshot.lock().await = router_infos(&routers);
        }

        Ok(router_infos(&routers))
    }
}

fn router_infos(
    routers: &BTreeMap<(Vec<u8>, Option<(u16, Vec<u8>)>), BTreeSet<u16>>,
) -> Vec<RouterInfo> {
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
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
    use bacnet_transport::loopback::LoopbackTransport;

    async fn send_router_announcement(
        transport: &LoopbackTransport,
        payload: &[u8],
        source: Option<NpduAddress>,
    ) {
        let npdu = Npdu {
            is_network_message: true,
            message_type: Some(
                bacnet_types::enums::NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw(),
            ),
            source,
            payload: Bytes::copy_from_slice(payload),
            ..Npdu::default()
        };
        let mut encoded = BytesMut::new();
        encode_npdu(&mut encoded, &npdu).unwrap();
        transport.send_broadcast(&encoded).await.unwrap();
    }

    #[tokio::test]
    async fn scoped_router_discovery_merges_repeated_network_claims() {
        let client_mac = MacAddr::from_slice(&[0x01]);
        let router_mac = MacAddr::from_slice(&[192, 0, 2, 10, 0xBA, 0xC1]);
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(client_mac, router_mac.clone());
        let mut router_rx = router_transport.start().await.unwrap();
        let router_task = tokio::spawn(async move {
            let request = router_rx.recv().await.expect("Who-Is-Router request");
            let request = decode_npdu(request.npdu).unwrap();
            assert!(request.is_network_message);
            assert_eq!(
                request.message_type,
                Some(bacnet_types::enums::NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(request.payload.as_ref(), &[0x07, 0xD1]);

            send_router_announcement(&router_transport, &[0x07, 0xD1, 0x03, 0xE8], None).await;
            send_router_announcement(&router_transport, &[0x03, 0xE8, 0x0B, 0xB8], None).await;
        });

        let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap();
        let routers = client
            .who_is_router_to_network(Some(2001), Duration::from_millis(50))
            .await
            .unwrap();
        router_task.await.unwrap();

        assert_eq!(routers.len(), 1);
        assert_eq!(routers[0].source_mac, router_mac);
        assert_eq!(routers[0].networks, vec![1000, 2001, 3000]);
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn malformed_router_announcement_is_a_decode_failure() {
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(MacAddr::from_slice(&[0x01]), MacAddr::from_slice(&[0x02]));
        let mut router_rx = router_transport.start().await.unwrap();
        let router_task = tokio::spawn(async move {
            router_rx.recv().await.expect("Who-Is-Router request");
            send_router_announcement(&router_transport, &[0x01], None).await;
        });

        let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap();
        let error = client
            .who_is_router_to_network(None, Duration::from_millis(50))
            .await
            .expect_err("malformed response must fail");
        assert!(matches!(error, Error::Decoding { .. }));
        router_task.await.unwrap();
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_observation_retains_partial_router_snapshot() {
        let router_mac = MacAddr::from_slice(&[192, 0, 2, 12, 0xBA, 0xC2]);
        let (client_transport, mut router_transport) =
            LoopbackTransport::pair(MacAddr::from_slice(&[0x01]), router_mac.clone());
        let mut router_rx = router_transport.start().await.unwrap();
        let router_task = tokio::spawn(async move {
            let request = router_rx.recv().await.expect("Who-Is-Router request");
            let request = decode_npdu(request.npdu).unwrap();
            assert!(request.payload.is_empty());
            send_router_announcement(&router_transport, &[0x0F, 0xA0], None).await;
        });

        let mut client = BACnetClient::start(ClientConfig::default(), client_transport)
            .await
            .unwrap();
        let result = timeout(
            Duration::from_millis(100),
            client.who_is_router_to_network(None, Duration::from_secs(10)),
        )
        .await;
        assert!(result.is_err(), "outer timeout must cancel the observation");
        router_task.await.unwrap();

        let snapshot = client.router_snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].source_mac, router_mac);
        assert_eq!(snapshot[0].networks, vec![4000]);
        client.stop().await.unwrap();
    }
}
