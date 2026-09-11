use super::*;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac,
};
use std::net::Ipv4Addr;
use tokio::time::{timeout, Duration};

struct ObserverTransport {
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
}

impl TransportPort for ObserverTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.rx
            .take()
            .ok_or_else(|| Error::Encoding("transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[0x01]
    }
}

fn received_npdu(npdu: Bytes, source_mac: &[u8]) -> ReceivedNpdu {
    ReceivedNpdu {
        npdu,
        source_mac: MacAddr::from_slice(source_mac),
        link_layer_group: false,
        data_attributes: Vec::new(),
        transport_meta: None,
        reply_tx: None,
    }
}

async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0x33; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn assert_sc_socket_closed_after_drop(ws_hub: &LoopbackWebSocket, context: &str) {
    timeout(Duration::from_secs(1), async {
        loop {
            match ws_hub.recv().await {
                Ok(data) => {
                    let msg = decode_sc_message(&data).unwrap();
                    assert_ne!(
                        msg.function,
                        ScFunction::HeartbeatAck,
                        "{context} must not leave SC answering heartbeats"
                    );
                }
                Err(_) => break,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{context} did not close the SC WebSocket"));

    let heartbeat = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id: 0x66,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &heartbeat);
    assert!(
        ws_hub.send(&buf).await.is_err(),
        "{context} must reject post-drop Heartbeat-Request on the closed socket"
    );
}

#[tokio::test]
async fn observer_reports_decode_failures_without_changing_apdu_delivery() {
    use crate::observer::{ApduDecode, ApduDirection, DecodeStage};

    let (transport_tx, transport_rx) = mpsc::channel(2);
    let (observer, mut events) = ApduObserver::new(4);
    let mut layer = NetworkLayer::with_observer(
        ObserverTransport {
            rx: Some(transport_rx),
        },
        observer,
    );
    let mut apdus = layer.start().await.unwrap();

    transport_tx
        .send(received_npdu(Bytes::from_static(&[0xff]), &[7]))
        .await
        .unwrap();

    let mut valid_npdu = BytesMut::new();
    encode_npdu(
        &mut valid_npdu,
        &Npdu {
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        },
    )
    .unwrap();
    transport_tx
        .send(received_npdu(valid_npdu.freeze(), &[8]))
        .await
        .unwrap();

    let malformed = events.recv().await.unwrap();
    assert_eq!(malformed.direction, ApduDirection::Inbound);
    assert_eq!(malformed.immediate_peer.as_slice(), &[7]);
    assert!(matches!(
        malformed.decode,
        ApduDecode::Error(ref error) if error.stage == DecodeStage::Npdu
    ));

    let decoded = events.recv().await.unwrap();
    assert!(matches!(decoded.decode, ApduDecode::Decoded(_)));
    assert_eq!(
        apdus.recv().await.unwrap().apdu,
        Bytes::from_static(&[0x10, 0x08])
    );

    layer.stop().await.unwrap();
}

#[tokio::test]
async fn observer_excludes_valid_network_control_messages() {
    let (transport_tx, transport_rx) = mpsc::channel(1);
    let (observer, mut events) = ApduObserver::new(2);
    let mut layer = NetworkLayer::with_observer(
        ObserverTransport {
            rx: Some(transport_rx),
        },
        observer,
    );
    let mut controls = layer.enable_network_control_receiver().unwrap();
    let _apdus = layer.start().await.unwrap();

    let mut encoded = BytesMut::new();
    encode_npdu(
        &mut encoded,
        &Npdu {
            is_network_message: true,
            message_type: Some(0),
            ..Npdu::default()
        },
    )
    .unwrap();
    transport_tx
        .send(received_npdu(encoded.freeze(), &[7]))
        .await
        .unwrap();

    timeout(Duration::from_secs(1), controls.recv())
        .await
        .expect("network control delivery timed out")
        .expect("network control receiver closed");
    assert!(timeout(Duration::from_millis(20), events.recv())
        .await
        .is_err());

    layer.stop().await.unwrap();
}

#[tokio::test]
async fn observer_reports_routed_outbound_and_stop_closes_receiver() {
    use crate::observer::{ApduDecode, ApduDirection};

    let (_transport_tx, transport_rx) = mpsc::channel(1);
    let (observer, mut events) = ApduObserver::new(2);
    let mut layer = NetworkLayer::with_observer(
        ObserverTransport {
            rx: Some(transport_rx),
        },
        observer,
    );

    layer
        .send_apdu_routed(
            &[0x10, 0x08],
            42,
            &[1, 2],
            &[9],
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    let outbound = events.recv().await.unwrap();
    assert_eq!(outbound.direction, ApduDirection::Outbound);
    assert_eq!(outbound.immediate_peer.as_slice(), &[9]);
    assert_eq!(outbound.routed_address.unwrap().network, 42);
    assert!(matches!(outbound.decode, ApduDecode::Decoded(_)));

    layer.stop().await.unwrap();
    assert_eq!(
        events.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    );
}

#[test]
fn disabled_observer_does_not_materialize_an_event_copy() {
    let (_transport_tx, transport_rx) = mpsc::channel(1);
    let layer = NetworkLayer::new(ObserverTransport {
        rx: Some(transport_rx),
    });

    layer.observe_outbound_with(&[9], || {
        panic!("disabled observer must not allocate or copy NPDU bytes")
    });
}

#[tokio::test]
async fn network_layer_drop_releases_sc_transport_socket() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let mut net =
        NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]));

    let hub_accept_task = tokio::spawn(async move {
        sc_hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = net.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    drop(net);

    assert_sc_socket_closed_after_drop(&ws_hub, "dropped NetworkLayer").await;
}

#[tokio::test]
async fn send_apdu_data_attributes_reach_sc_data_options() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let hub_vmac = [0x10; 6];
    let dest_vmac: Vmac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    let mut net =
        NetworkLayer::new(ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]));
    let data_attributes = vec![
        DataAttribute {
            option_type: 1,
            must_understand: true,
            data: Vec::new(),
        },
        DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34, 0x56],
        },
    ];

    let hub_accept_task = tokio::spawn(async move {
        sc_hub_accept(&ws_hub, hub_vmac).await;
        ws_hub
    });

    let _rx = net.start().await.unwrap();
    let ws_hub = hub_accept_task.await.unwrap();

    let apdu = Bytes::from_static(&[0x10, 0x08]);
    net.send_apdu_with_data_attributes(
        &apdu,
        &dest_vmac,
        false,
        NetworkPriority::NORMAL,
        &data_attributes,
    )
    .await
    .unwrap();

    let data = ws_hub.recv().await.unwrap();
    let msg = decode_sc_message(&data).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(dest_vmac));
    assert_eq!(msg.data_options.len(), 2);
    assert_eq!(msg.data_options[0].option_type, 1);
    assert!(msg.data_options[0].must_understand);
    assert_eq!(msg.data_options[1].option_type, 31);
    assert!(!msg.data_options[1].must_understand);
    assert_eq!(msg.data_options[1].data, vec![0x12, 0x34, 0x56]);

    let npdu = decode_npdu(msg.payload).unwrap();
    assert_eq!(npdu.payload, apdu);
    assert!(!npdu.expecting_reply);
    assert_eq!(npdu.priority, NetworkPriority::NORMAL);

    net.stop().await.unwrap();
}

#[tokio::test]
async fn end_to_end_who_is() {
    use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, UnconfirmedRequest};
    use bacnet_types::enums::UnconfirmedServiceChoice;

    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let mut net_a = NetworkLayer::new(transport_a);
    let mut net_b = NetworkLayer::new(transport_b);

    let _rx_a = net_a.start().await.unwrap();
    let mut rx_b = net_b.start().await.unwrap();

    let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

    net_a
        .send_apdu(&apdu_buf, net_b.local_mac(), false, NetworkPriority::NORMAL)
        .await
        .unwrap();

    let received = timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("Timed out waiting for APDU")
        .expect("Channel closed");

    let decoded_apdu = decode_apdu(received.apdu.clone()).unwrap();
    match decoded_apdu {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(req.service_choice, UnconfirmedServiceChoice::WHO_IS);
            assert!(req.service_request.is_empty());
        }
        other => panic!("Expected UnconfirmedRequest, got {:?}", other),
    }

    net_a.stop().await.unwrap();
    net_b.stop().await.unwrap();
}

#[test]
fn global_broadcast_npdu_has_dnet_ffff() {
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
    use bacnet_types::enums::NetworkPriority;

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xAA]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 0xFFFF);
    assert!(dest.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 255);
}

#[test]
fn transport_accessor() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let net = NetworkLayer::new(transport);
    let mac = net.transport().local_mac();
    assert_eq!(mac.len(), 6);
}

#[test]
fn routed_send_encodes_dnet_dadr() {
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
    use bacnet_types::enums::NetworkPriority;

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: true,
        priority: NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 100,
            mac_address: MacAddr::from_slice(&[1, 2, 3, 4, 5, 6]),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xAA, 0xBB]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 100);
    assert_eq!(dest.mac_address.as_slice(), &[1, 2, 3, 4, 5, 6]);
    assert_eq!(decoded.hop_count, 255);
    assert!(decoded.expecting_reply);
}

#[test]
fn broadcast_to_network_encodes_specific_dnet() {
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
    use bacnet_types::enums::NetworkPriority;

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 42,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xCC]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 42);
    assert!(dest.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 255);
    assert!(!decoded.expecting_reply);
}
