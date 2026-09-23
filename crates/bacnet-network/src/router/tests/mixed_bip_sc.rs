use super::*;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportHealth, TransportHealthState};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac,
};
use std::net::Ipv4Addr;
use std::net::SocketAddrV4;
use tokio::time::{timeout, Duration};

enum MixedTestTransport {
    Bip(BipTransport),
    Sc(ScTransport<LoopbackWebSocket>),
}

impl TransportPort for MixedTestTransport {
    fn transport_kind(&self) -> &'static str {
        match self {
            Self::Bip(port) => port.transport_kind(),
            Self::Sc(port) => port.transport_kind(),
        }
    }

    fn topology_id(&self) -> Option<String> {
        match self {
            Self::Bip(port) => port.topology_id(),
            Self::Sc(port) => port.topology_id(),
        }
    }

    fn health(&self) -> TransportHealth {
        match self {
            Self::Bip(port) => port.health(),
            Self::Sc(port) => port.health(),
        }
    }

    fn health_changes(&self) -> Option<watch::Receiver<TransportHealth>> {
        match self {
            Self::Bip(port) => port.health_changes(),
            Self::Sc(port) => port.health_changes(),
        }
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        match self {
            Self::Bip(port) => port.start().await,
            Self::Sc(port) => port.start().await,
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match self {
            Self::Bip(port) => port.stop().await,
            Self::Sc(port) => port.stop().await,
        }
    }

    fn abort(&mut self) {
        match self {
            Self::Bip(port) => port.abort(),
            Self::Sc(port) => port.abort(),
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(port) => port.send_unicast(npdu, mac).await,
            Self::Sc(port) => port.send_unicast(npdu, mac).await,
        }
    }

    async fn send_unicast_with_data_attributes<'a>(
        &'a self,
        npdu: &'a [u8],
        mac: &'a [u8],
        data_attributes: &'a [DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(port) => {
                port.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
            Self::Sc(port) => {
                port.send_unicast_with_data_attributes(npdu, mac, data_attributes)
                    .await
            }
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        match self {
            Self::Bip(port) => port.send_broadcast(npdu).await,
            Self::Sc(port) => port.send_broadcast(npdu).await,
        }
    }

    async fn send_broadcast_with_data_attributes<'a>(
        &'a self,
        npdu: &'a [u8],
        data_attributes: &'a [DataAttribute],
    ) -> Result<(), Error> {
        match self {
            Self::Bip(port) => {
                port.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
            Self::Sc(port) => {
                port.send_broadcast_with_data_attributes(npdu, data_attributes)
                    .await
            }
        }
    }

    fn local_mac(&self) -> &[u8] {
        match self {
            Self::Bip(port) => port.local_mac(),
            Self::Sc(port) => port.local_mac(),
        }
    }

    fn max_apdu_length(&self) -> u16 {
        match self {
            Self::Bip(port) => port.max_apdu_length(),
            Self::Sc(port) => port.max_apdu_length(),
        }
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        match self {
            Self::Bip(port) => port.is_broadcast_mac(mac),
            Self::Sc(port) => port.is_broadcast_mac(mac),
        }
    }
}

async fn accept_sc(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let request = decode_sc_message(&ws_hub.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);

    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476_u16.to_be_bytes());
    payload.extend_from_slice(&1476_u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: request.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut encoded = BytesMut::new();
    encode_sc_message(&mut encoded, &accept);
    ws_hub.send(&encoded).await.unwrap();
}

async fn next_sc_npdu(ws_hub: &LoopbackWebSocket, apdu: &[u8]) -> Npdu {
    timeout(Duration::from_secs(2), async {
        loop {
            let message = decode_sc_message(&ws_hub.recv().await.unwrap()).unwrap();
            if message.function != ScFunction::EncapsulatedNpdu {
                continue;
            }
            let npdu = decode_npdu(message.payload).unwrap();
            if npdu.payload.as_ref() == apdu {
                return npdu;
            }
        }
    })
    .await
    .expect("timed out waiting for SC NPDU")
}

async fn next_bip_npdu(receiver: &mut mpsc::Receiver<ReceivedNpdu>, apdu: &[u8]) -> ReceivedNpdu {
    timeout(Duration::from_secs(2), async {
        loop {
            let received = receiver.recv().await.expect("B/IP receive task ended");
            let decoded = decode_npdu(received.npdu.clone()).unwrap();
            if decoded.payload.as_ref() == apdu {
                return received;
            }
        }
    })
    .await
    .expect("timed out waiting for B/IP NPDU")
}

fn encode(npdu: &Npdu) -> BytesMut {
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, npdu).unwrap();
    encoded
}

fn bip_identity_mac(identity: &str) -> Vec<u8> {
    if let Ok(address) = identity.parse::<SocketAddrV4>() {
        let mut mac = address.ip().octets().to_vec();
        mac.extend_from_slice(&address.port().to_be_bytes());
        return mac;
    }
    identity
        .split(':')
        .map(|octet| u8::from_str_radix(octet, 16).unwrap())
        .collect()
}

#[tokio::test]
async fn wildcard_bip_counter_identity_uses_resolved_local_mac() {
    let router_bip = BipTransport::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST);
    let (sc_client, sc_hub) = LoopbackWebSocket::pair();
    let accept = tokio::spawn(async move {
        accept_sc(&sc_hub, [0xA0; 6]).await;
        sc_hub
    });
    let ports = vec![
        RouterPort {
            transport: MixedTestTransport::Bip(router_bip),
            network_number: 1000,
        },
        RouterPort {
            transport: MixedTestTransport::Sc(
                ScTransport::new(sc_client, [0x02; 6]).with_device_uuid([2; 16]),
            ),
            network_number: 2000,
        },
    ];
    let (mut router, _local_rx) = BACnetRouter::start(ports).await.unwrap();
    let _sc_hub = accept.await.unwrap();
    let mac = bip_identity_mac(&router.port_counters()[0].identity);
    assert_eq!(mac.len(), 6);
    assert_ne!(&mac[..4], &[0, 0, 0, 0]);
    assert_ne!(u16::from_be_bytes([mac[4], mac[5]]), 0);
    router.stop().await;
}

#[tokio::test]
async fn bip_and_sc_route_unicast_both_ways_and_global_broadcast() {
    let mut bip_device = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut bip_device_rx = bip_device.start().await.unwrap();
    let bip_device_mac = MacAddr::from_slice(bip_device.local_mac());

    let router_bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let (sc_client, sc_hub) = LoopbackWebSocket::pair();
    let sc_router_vmac: Vmac = [0x02; 6];
    let sc_device_vmac: Vmac = [0x44; 6];
    let hub_vmac: Vmac = [0xA0; 6];
    let accept = tokio::spawn(async move {
        accept_sc(&sc_hub, hub_vmac).await;
        sc_hub
    });
    let ports = vec![
        RouterPort {
            transport: MixedTestTransport::Bip(router_bip),
            network_number: 1000,
        },
        RouterPort {
            transport: MixedTestTransport::Sc(
                ScTransport::new(sc_client, sc_router_vmac).with_device_uuid([2; 16]),
            ),
            network_number: 2000,
        },
    ];
    let (mut router, _local_rx) = BACnetRouter::start(ports).await.unwrap();
    let sc_hub = accept.await.unwrap();
    let router_bip_mac = bip_identity_mac(&router.port_counters()[0].identity);

    let bip_to_sc_apdu = [0x10, 0x08, 0x01];
    let bip_to_sc = Npdu {
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&sc_device_vmac),
        }),
        hop_count: 7,
        payload: Bytes::copy_from_slice(&bip_to_sc_apdu),
        ..Npdu::default()
    };
    bip_device
        .send_unicast(&encode(&bip_to_sc), &router_bip_mac)
        .await
        .unwrap();
    let forwarded_sc = next_sc_npdu(&sc_hub, &bip_to_sc_apdu).await;
    assert!(forwarded_sc.destination.is_none());
    let source = forwarded_sc.source.unwrap();
    assert_eq!(source.network, 1000);
    assert_eq!(source.mac_address, bip_device_mac);

    let sc_to_bip_apdu = [0x10, 0x08, 0x02];
    let sc_to_bip = Npdu {
        destination: Some(NpduAddress {
            network: 1000,
            mac_address: bip_device_mac.clone(),
        }),
        hop_count: 9,
        payload: Bytes::copy_from_slice(&sc_to_bip_apdu),
        ..Npdu::default()
    };
    let inbound_sc = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1200,
        originating_vmac: Some(hub_vmac),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: encode(&sc_to_bip).freeze(),
    };
    let mut inbound_sc_wire = BytesMut::new();
    encode_sc_message(&mut inbound_sc_wire, &inbound_sc);
    sc_hub.send(&inbound_sc_wire).await.unwrap();
    let received_bip = next_bip_npdu(&mut bip_device_rx, &sc_to_bip_apdu).await;
    let forwarded_bip = decode_npdu(received_bip.npdu).unwrap();
    assert!(forwarded_bip.destination.is_none());
    let source = forwarded_bip.source.unwrap();
    assert_eq!(source.network, 2000);
    assert_eq!(source.mac_address.as_slice(), &hub_vmac);

    let broadcast_apdu = [0x10, 0x08, 0x03];
    let broadcast = Npdu {
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        hop_count: 5,
        payload: Bytes::copy_from_slice(&broadcast_apdu),
        ..Npdu::default()
    };
    bip_device
        .send_unicast(&encode(&broadcast), &router_bip_mac)
        .await
        .unwrap();
    let forwarded_broadcast = next_sc_npdu(&sc_hub, &broadcast_apdu).await;
    assert_eq!(forwarded_broadcast.destination.unwrap().network, 0xFFFF);
    assert_eq!(forwarded_broadcast.hop_count, 4);
    let source = forwarded_broadcast.source.unwrap();
    assert_eq!(source.network, 1000);
    assert_eq!(source.mac_address, bip_device_mac);

    let health = router.port_health();
    assert_eq!(health[0].health.state, TransportHealthState::Up);
    assert_eq!(health[1].health.state, TransportHealthState::Up);
    assert_eq!(router.port_counters()[0].transport_kind, "bip");
    assert_eq!(router.port_counters()[1].transport_kind, "sc");

    router.stop().await;
    bip_device.stop().await.unwrap();
}
