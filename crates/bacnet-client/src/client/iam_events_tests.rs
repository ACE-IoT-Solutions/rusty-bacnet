use super::*;
use bacnet_encoding::apdu::{encode_apdu, UnconfirmedRequest};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::{BvlcFunction, ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

struct InjectedTransport {
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    local_mac: MacAddr,
}

impl TransportPort for InjectedTransport {
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
        &self.local_mac
    }
}

fn i_am_npdu(instance: u32, source: Option<NpduAddress>) -> Bytes {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 42,
    }
    .encode(&mut service_request);
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            destination: source.as_ref().map(|_| NpduAddress {
                network: 0xFFFF,
                mac_address: MacAddr::new(),
            }),
            source,
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu_buf.freeze()
}

fn event(instance: u32) -> IAmEvent {
    let object_identifier = ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    IAmEvent {
        device_instance: instance,
        object_identifier,
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 42,
        udp_source_ip: None,
        udp_source_port: None,
        source_mac: MacAddr::from_slice(&[0x01]),
        source_network: None,
        source_address: None,
        bvlc_function: None,
        forwarded_from_ip: None,
        forwarded_from_port: None,
        timestamp: Instant::now(),
    }
}

#[tokio::test]
async fn duplicate_i_am_observations_preserve_transport_and_routing_provenance() {
    let (tx, rx) = mpsc::channel(4);
    let mut client = BACnetClient::generic_builder()
        .transport(InjectedTransport {
            rx: Some(rx),
            local_mac: MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
        })
        .build()
        .await
        .unwrap();
    let mut events = client.iam_events();
    let instance = 1001;

    tx.send(ReceivedNpdu {
        npdu: i_am_npdu(instance, None),
        source_mac: MacAddr::from_slice(&[192, 0, 2, 10, 0xBA, 0xC1]),
        link_layer_group: true,
        data_attributes: Vec::new(),
        transport_meta: Some(TransportMeta {
            bvlc_function: BvlcFunction::ORIGINAL_BROADCAST_NPDU.to_raw(),
            udp_source_ip: [192, 0, 2, 10],
            udp_source_port: 0xBAC1,
            forwarded_from_ip: None,
            forwarded_from_port: None,
        }),
        reply_tx: None,
    })
    .await
    .unwrap();

    let sadr = MacAddr::from_slice(&[0x44, 0x55]);
    tx.send(ReceivedNpdu {
        npdu: i_am_npdu(
            instance,
            Some(NpduAddress {
                network: 200,
                mac_address: sadr.clone(),
            }),
        ),
        source_mac: MacAddr::from_slice(&[203, 0, 113, 7, 0xBA, 0xC3]),
        link_layer_group: true,
        data_attributes: Vec::new(),
        transport_meta: Some(TransportMeta {
            bvlc_function: BvlcFunction::FORWARDED_NPDU.to_raw(),
            udp_source_ip: [198, 51, 100, 5],
            udp_source_port: 0xBAC2,
            forwarded_from_ip: Some([203, 0, 113, 7]),
            forwarded_from_port: Some(0xBAC3),
        }),
        reply_tx: None,
    })
    .await
    .unwrap();

    let first = timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    let second = timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.device_instance, instance);
    assert_eq!(second.device_instance, instance);
    assert_eq!(first.udp_source_port, Some(0xBAC1));
    assert_eq!(first.forwarded_from_ip, None);
    assert_eq!(
        second.bvlc_function,
        Some(BvlcFunction::FORWARDED_NPDU.to_raw())
    );
    assert_eq!(second.udp_source_ip, Some([198, 51, 100, 5]));
    assert_eq!(second.forwarded_from_ip, Some([203, 0, 113, 7]));
    assert_eq!(second.forwarded_from_port, Some(0xBAC3));
    assert_eq!(second.source_network, Some(200));
    assert_eq!(second.source_address, Some(sadr));
    assert!(second.timestamp >= first.timestamp);
    assert_eq!(client.discovered_devices().await.len(), 1);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn i_am_event_channel_is_bounded_and_reports_lag() {
    let (_tx, rx) = mpsc::channel(1);
    let client = BACnetClient::generic_builder()
        .transport(InjectedTransport {
            rx: Some(rx),
            local_mac: MacAddr::from_slice(&[0x01]),
        })
        .build()
        .await
        .unwrap();
    let mut events = client.iam_events();
    for instance in 0..=IAM_EVENT_CHANNEL_CAPACITY {
        client.iam_tx.send(event(instance as u32)).unwrap();
    }
    assert_eq!(
        events.recv().await,
        Err(broadcast::error::RecvError::Lagged(1))
    );
    assert_eq!(events.recv().await.unwrap().device_instance, 1);
}

#[tokio::test]
async fn non_bip_i_am_has_no_ipv4_transport_metadata() {
    let (client_transport, sender_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut events = client.iam_events();
    sender_transport
        .send_unicast(&i_am_npdu(2002, None), client.local_mac())
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.source_mac.as_slice(), &[0x02]);
    assert_eq!(event.udp_source_ip, None);
    assert_eq!(event.bvlc_function, None);
    assert_eq!(event.forwarded_from_ip, None);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn initial_i_am_receiver_is_reserved_and_takeable_once() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    assert!(client.take_initial_iam_receiver().is_some());
    assert!(client.take_initial_iam_receiver().is_none());
    client.stop().await.unwrap();
}
