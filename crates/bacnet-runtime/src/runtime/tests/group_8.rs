fn passive_cov_npdu(
    process_id: u32,
    delivery: COVNotificationDelivery,
    source: Option<NpduAddress>,
) -> Bytes {
    let notification = COVNotificationRequest {
        subscriber_process_identifier: process_id,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 4_020_008).unwrap(),
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17).unwrap(),
        time_remaining: 47,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(2),
            value: vec![0x44, 0x41, 0xfc, 0x00, 0x00],
            priority: Some(8),
        }],
    };
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    let apdu = match delivery {
        COVNotificationDelivery::Confirmed => Apdu::ConfirmedRequest(apdu::ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 73,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        COVNotificationDelivery::Unconfirmed => Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
    };
    let mut apdu_bytes = BytesMut::new();
    encode_apdu(&mut apdu_bytes, &apdu).unwrap();
    let mut npdu_bytes = BytesMut::new();
    encode_npdu(
        &mut npdu_bytes,
        &Npdu {
            source,
            payload: apdu_bytes.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu_bytes.freeze()
}

async fn wait_for_unsolicited_cov(
    runtime: &BacnetRuntime,
    process_id: u32,
) -> (crate::RuntimeEvent, crate::UnsolicitedCovNotification) {
    timeout(Duration::from_secs(2), async {
        loop {
            for event in runtime
                .next_events(8, Duration::from_millis(20))
                .await
                .events
            {
                if let EventKind::UnsolicitedCovNotification { notification } = &event.kind {
                    if notification.subscriber_process_identifier == process_id {
                        return (event.clone(), notification.clone());
                    }
                }
            }
        }
    })
    .await
    .expect("runtime did not project passive COV notification")
}

#[tokio::test]
async fn passive_cov_wire_projection_preserves_direct_and_routed_provenance_and_one_ack() {
    let attachment_id = AttachmentId::from(108);
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(108, "passive-wire-cov")],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let destination = runtime
        .inner
        .registry
        .read()
        .await
        .transport(attachment_id)
        .unwrap()
        .local_mac()
        .to_vec();
    let _ = runtime.next_events(8, Duration::ZERO).await;

    let mut sender = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut sender_rx = sender.start().await.unwrap();
    let immediate_source = MacAddr::from_slice(sender.local_mac());

    sender
        .send_unicast(
            &passive_cov_npdu(108_001, COVNotificationDelivery::Unconfirmed, None),
            &destination,
        )
        .await
        .unwrap();
    let (event, notification) = wait_for_unsolicited_cov(&runtime, 108_001).await;
    assert_eq!(event.attachment_id, Some(attachment_id));
    assert_eq!(notification.delivery, COVNotificationDelivery::Unconfirmed);
    assert_eq!(notification.source_mac, immediate_source);
    assert_eq!(notification.source_network, None);
    assert_eq!(notification.source_address, None);
    assert_eq!(notification.values[0].array_index, Some(2));
    assert_eq!(notification.values[0].priority, Some(8));

    let routed_source = NpduAddress {
        network: 2001,
        mac_address: MacAddr::from_slice(&[0, 7, 0xaa]),
    };
    sender
        .send_unicast(
            &passive_cov_npdu(
                108_002,
                COVNotificationDelivery::Confirmed,
                Some(routed_source.clone()),
            ),
            &destination,
        )
        .await
        .unwrap();
    let (event, notification) = wait_for_unsolicited_cov(&runtime, 108_002).await;
    assert_eq!(event.attachment_id, Some(attachment_id));
    assert_eq!(notification.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(notification.source_mac, immediate_source);
    assert_eq!(notification.source_network, Some(routed_source.network));
    assert_eq!(
        notification.source_address,
        Some(routed_source.mac_address.clone())
    );

    let ack_wire = timeout(Duration::from_secs(2), sender_rx.recv())
        .await
        .expect("confirmed COV acknowledgement timed out")
        .expect("sender receive channel closed");
    let ack_npdu = decode_npdu(ack_wire.npdu).unwrap();
    assert_eq!(ack_npdu.destination, Some(routed_source));
    assert_eq!(
        apdu::decode_apdu(ack_npdu.payload).unwrap(),
        Apdu::SimpleAck(SimpleAck {
            invoke_id: 73,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        })
    );
    assert!(timeout(Duration::from_millis(100), sender_rx.recv())
        .await
        .is_err());
    assert!(!runtime
        .next_events(8, Duration::from_millis(50))
        .await
        .events
        .iter()
        .any(|event| matches!(
            event.kind,
            EventKind::UnsolicitedCovNotification { ref notification }
                if notification.subscriber_process_identifier == 108_002
        )));

    sender.stop().await.unwrap();
    runtime.stop().await.unwrap();
    assert!(runtime.inner.cov_pumps.lock().await.is_empty());
    assert_eq!(runtime.inner.supervisor.task_count(), 0);
}
