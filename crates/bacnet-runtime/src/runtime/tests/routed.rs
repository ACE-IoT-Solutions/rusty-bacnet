use super::*;
use crate::batch::{DevicePath, WriteBatch, WriteBatchItem};
use bacnet_encoding::apdu::{self, Apdu, ComplexAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bytes::{Bytes, BytesMut};
use std::net::Ipv4Addr;

fn property_target(router_mac: Vec<u8>, dadr: Vec<u8>) -> PropertyTarget {
    PropertyTarget::routed(
        router_mac,
        200,
        dadr,
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 4).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    )
}

async fn send_routed_reply(peer: &BipTransport, runtime_mac: &[u8], dadr: &[u8], apdu: Apdu) {
    let mut apdu_bytes = BytesMut::new();
    encode_apdu(&mut apdu_bytes, &apdu).unwrap();
    let mut npdu_bytes = BytesMut::new();
    encode_npdu(
        &mut npdu_bytes,
        &Npdu {
            source: Some(NpduAddress {
                network: 200,
                mac_address: MacAddr::from_slice(dadr),
            }),
            payload: apdu_bytes.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    peer.send_unicast(&npdu_bytes, runtime_mac).await.unwrap();
}

#[tokio::test]
async fn routed_read_preserves_dnet_dadr_and_outcome_path() {
    let mut peer = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut incoming = peer.start().await.unwrap();
    let router_mac = peer.local_mac().to_vec();
    let dadr = vec![0, 42];
    let expected_path = DevicePath::Routed {
        ingress_mac: router_mac.clone(),
        dnet: 200,
        dadr: dadr.clone(),
    };
    let responder = tokio::spawn(async move {
        let frame = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(frame.npdu).unwrap();
        let destination = npdu.destination.unwrap();
        assert_eq!(destination.network, 200);
        assert_eq!(destination.mac_address.as_slice(), &[0, 42]);
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected ReadProperty request")
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY
        );
        let property = ReadPropertyRequest::decode(&request.service_request).unwrap();
        let ack = ReadPropertyACK {
            object_identifier: property.object_identifier,
            property_identifier: property.property_identifier,
            property_array_index: property.property_array_index,
            property_value: vec![0x44, 0x41, 0x20, 0, 0],
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        send_routed_reply(
            &peer,
            frame.source_mac.as_slice(),
            &[0, 42],
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from(service_ack.to_vec()),
            }),
        )
        .await;
        peer.stop().await.unwrap();
    });

    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let outcomes = runtime
        .read_batch(ReadBatch {
            items: vec![ReadBatchItem {
                input_index: 4,
                attachment_id: AttachmentId::from(1),
                target: property_target(router_mac, dadr),
            }],
            timeout: Duration::from_secs(2),
        })
        .await
        .unwrap()
        .result()
        .await
        .unwrap();
    assert_eq!(outcomes[0].path.as_ref(), Some(&expected_path));
    assert_eq!(
        outcomes[0].raw_value.as_deref(),
        Some(&[0x44, 0x41, 0x20, 0, 0][..])
    );
    assert!(outcomes[0].error.is_none());
    responder.await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn routed_write_preserves_path_and_priority() {
    let mut peer = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut incoming = peer.start().await.unwrap();
    let router_mac = peer.local_mac().to_vec();
    let dadr = vec![7];
    let expected_path = DevicePath::Routed {
        ingress_mac: router_mac.clone(),
        dnet: 200,
        dadr: dadr.clone(),
    };
    let responder = tokio::spawn(async move {
        let frame = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(frame.npdu).unwrap();
        assert_eq!(npdu.destination.as_ref().unwrap().network, 200);
        assert_eq!(
            npdu.destination.as_ref().unwrap().mac_address.as_slice(),
            &[7]
        );
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected WriteProperty request")
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::WRITE_PROPERTY
        );
        let property = WritePropertyRequest::decode(&request.service_request).unwrap();
        assert_eq!(property.priority, Some(8));
        send_routed_reply(
            &peer,
            frame.source_mac.as_slice(),
            &[7],
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            }),
        )
        .await;
        peer.stop().await.unwrap();
    });

    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let outcomes = runtime
        .write_batch(WriteBatch {
            items: vec![WriteBatchItem {
                input_index: 5,
                attachment_id: AttachmentId::from(1),
                target: property_target(router_mac, dadr),
                raw_value: vec![0x44, 0x42, 0x90, 0, 0],
                priority: Some(8),
            }],
            timeout: Duration::from_secs(2),
        })
        .await
        .unwrap()
        .result()
        .await
        .unwrap();
    assert_eq!(outcomes[0].path.as_ref(), Some(&expected_path));
    assert!(outcomes[0].written);
    assert!(outcomes[0].error.is_none());
    responder.await.unwrap();
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_routed_paths_are_rejected_before_operation_admission() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let invalid = [
        DevicePath::Direct { mac: vec![1; 5] },
        DevicePath::Routed {
            ingress_mac: vec![1; 5],
            dnet: 1,
            dadr: vec![1],
        },
        DevicePath::Routed {
            ingress_mac: vec![1; 6],
            dnet: 0,
            dadr: vec![1],
        },
        DevicePath::Routed {
            ingress_mac: vec![1; 6],
            dnet: u16::MAX,
            dadr: vec![1],
        },
        DevicePath::Routed {
            ingress_mac: vec![1; 6],
            dnet: 1,
            dadr: Vec::new(),
        },
        DevicePath::Routed {
            ingress_mac: vec![1; 6],
            dnet: 1,
            dadr: vec![1; 19],
        },
    ];
    for path in invalid {
        let mut target = property_target(vec![1; 6], vec![1]);
        target.path = path;
        let result = runtime
            .read_batch(ReadBatch::new(vec![ReadBatchItem {
                input_index: 0,
                attachment_id: AttachmentId::from(1),
                target,
            }]))
            .await;
        let error = match result {
            Ok(_) => panic!("invalid routed path was admitted"),
            Err(error) => error,
        };
        assert_eq!(error.code, crate::ErrorCode::InvalidConfig);
        assert_eq!(runtime.inner.active_operations.load(Ordering::Acquire), 0);
    }
    runtime.stop().await.unwrap();
}
