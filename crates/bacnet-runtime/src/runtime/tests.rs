use super::*;
use crate::batch::{PropertyTarget, ReadBatchItem};
use crate::config::{BipConfig, TransportConfig};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use std::time::Duration;
use tokio::net::UdpSocket;

mod routed;

fn attachment(id: u128, port: u16) -> AttachmentConfig {
    AttachmentConfig {
        id: AttachmentId::from(id),
        label: format!("bip-{id}"),
        transport: TransportConfig::Bip(BipConfig {
            interface: "127.0.0.1".to_owned(),
            port,
            broadcast: Some("127.255.255.255".to_owned()),
            ..BipConfig::default()
        }),
    }
}

#[test]
fn config_rejects_non_bip_transports() {
    let config = RuntimeConfig {
        attachments: vec![AttachmentConfig {
            id: AttachmentId::from(1),
            label: "sc".to_owned(),
            transport: TransportConfig::Sc(crate::ScConfig {
                primary_hub: "wss://example.test".to_owned(),
                failover_hubs: Vec::new(),
                local_vmac: [1, 2, 3, 4, 5, 6],
                device_uuid: [1; 16],
                ca_cert: None,
                client_cert: None,
                client_key: None,
                heartbeat_interval_ms: 30_000,
                heartbeat_timeout_ms: 60_000,
                reconnect_initial_delay_ms: 1_000,
                reconnect_max_delay_ms: 60_000,
                reconnect_max_retries: 3,
                reconnect_forever: false,
            }),
        }],
        ..RuntimeConfig::default()
    };
    let error = config.validate().unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::UnsupportedTransport);
}

#[test]
fn config_rejects_a_zero_bbmd_port_before_start() {
    let mut item = attachment(1, 0);
    let TransportConfig::Bip(config) = &mut item.transport else {
        unreachable!()
    };
    config.bbmd_address = Some("127.0.0.1:0".to_owned());
    config.foreign_device_ttl = Some(60);
    let error = RuntimeConfig {
        attachments: vec![item],
        ..RuntimeConfig::default()
    }
    .validate()
    .unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::InvalidConfig);
}

#[tokio::test]
async fn lifecycle_health_stop_and_restart() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let health = runtime.health().await;
    assert!(health.accepting_commands);
    assert_eq!(health.attachments.len(), 1);
    assert_eq!(health.attachments[0].state, AttachmentState::Running);
    assert_eq!(health.attachments[0].local_mac.as_ref().unwrap().len(), 6);

    let report = runtime.stop().await.unwrap();
    assert!(report.newly_stopped);
    assert!(!runtime.health().await.accepting_commands);
    assert!(!runtime.stop().await.unwrap().newly_stopped);
}

#[tokio::test]
async fn bind_failure_rolls_back_already_started_attachments() {
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let result = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, port), attachment(2, port)],
        ..RuntimeConfig::default()
    })
    .await;
    let error = match result {
        Ok(runtime) => {
            runtime.stop().await.unwrap();
            panic!("duplicate B/IP bind unexpectedly succeeded")
        }
        Err(error) => error,
    };
    assert_eq!(error.code, crate::ErrorCode::BindConflict);
}

#[tokio::test]
async fn reconcile_identical_configuration_is_idempotent() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let report = runtime.reconcile(1, Vec::new()).await.unwrap();
    assert!(report.idempotent);
    assert_eq!(report.generation, 1);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn reconcile_rejects_while_a_batch_operation_is_active() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let permit = runtime
        .inner
        .operation_slots
        .clone()
        .try_acquire_owned()
        .unwrap();
    let id = OperationId(98);
    runtime
        .inner
        .active_operations
        .fetch_add(1, Ordering::AcqRel);
    runtime.inner.operations.lock().unwrap().insert(id, None);
    let guard = OperationGuard::new(id, Arc::clone(&runtime.inner), permit);

    let error = runtime.reconcile(2, Vec::new()).await.unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::Backpressure);

    drop(guard);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn failed_reconcile_restores_the_previous_attachment_set() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let conflicting_port = probe.local_addr().unwrap().port();
    drop(probe);

    let error = runtime
        .reconcile(
            2,
            vec![
                attachment(2, conflicting_port),
                attachment(3, conflicting_port),
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::BindConflict);
    let health = runtime.health().await;
    assert!(health.accepting_commands);
    assert_eq!(health.attachments.len(), 1);
    assert_eq!(health.attachments[0].id, AttachmentId::from(1));
    assert_eq!(health.attachments[0].state, AttachmentState::Running);
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn failed_stop_retains_attachments_and_can_be_retried() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        shutdown_timeout: Duration::from_millis(10),
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let managed = runtime
        .inner
        .attachments
        .lock()
        .await
        .get(&AttachmentId::from(1))
        .unwrap()
        .clone();
    let client_guard = managed.client.lock().await;
    let stopping = runtime.clone();
    let stop_task = tokio::spawn(async move { stopping.stop().await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(client_guard);

    assert_eq!(
        stop_task.await.unwrap().unwrap_err().code,
        crate::ErrorCode::DeadlineExceeded
    );
    let health = runtime.health().await;
    assert!(!health.accepting_commands);
    assert_eq!(health.attachments.len(), 1);

    let retry = runtime.stop().await.unwrap();
    assert!(!retry.newly_stopped);
    assert_eq!(retry.remaining_attachments, 0);
    assert!(runtime.health().await.attachments.is_empty());
}

#[tokio::test]
async fn missing_attachment_batch_preserves_input_order() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let id = AttachmentId::from(42);
    let target = || {
        PropertyTarget::new(
            [127, 0, 0, 1, 0xba, 0xc0],
            ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 1).unwrap(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
    };
    let operation = runtime
        .read_batch(ReadBatch {
            items: vec![
                ReadBatchItem {
                    input_index: 9,
                    attachment_id: id,
                    target: target(),
                },
                ReadBatchItem {
                    input_index: 3,
                    attachment_id: id,
                    target: target(),
                },
            ],
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    let outcomes = operation.result().await.unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|item| item.input_index)
            .collect::<Vec<_>>(),
        [9, 3]
    );
    assert!(outcomes.iter().all(|item| item.error.is_some()));
    runtime.stop().await.unwrap();
}

#[tokio::test]
async fn dropping_an_operation_guard_releases_its_slot() {
    let runtime = BacnetRuntime::start(RuntimeConfig::default())
        .await
        .unwrap();
    let permit = runtime
        .inner
        .operation_slots
        .clone()
        .try_acquire_owned()
        .unwrap();
    runtime
        .inner
        .active_operations
        .fetch_add(1, Ordering::AcqRel);
    let id = OperationId(99);
    runtime.inner.operations.lock().unwrap().insert(id, None);
    let guard = OperationGuard::new(id, Arc::clone(&runtime.inner), permit);
    assert_eq!(runtime.inner.active_operations.load(Ordering::Acquire), 1);
    drop(guard);
    assert_eq!(runtime.inner.active_operations.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn stop_aborts_and_joins_an_inflight_batch() {
    let runtime = BacnetRuntime::start(RuntimeConfig {
        attachments: vec![attachment(1, 0)],
        ..RuntimeConfig::default()
    })
    .await
    .unwrap();
    let operation = runtime
        .read_batch(ReadBatch {
            items: vec![ReadBatchItem {
                input_index: 0,
                attachment_id: AttachmentId::from(1),
                target: PropertyTarget::new(
                    [127, 0, 0, 1, 0, 1],
                    ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 1).unwrap(),
                    PropertyIdentifier::OBJECT_NAME,
                    None,
                ),
            }],
            timeout: Duration::from_secs(60),
        })
        .await
        .unwrap();

    let report = runtime.stop().await.unwrap();
    assert_eq!(report.tasks_aborted, 1);
    assert_eq!(runtime.health().await.operation_count, 0);
    assert_eq!(
        operation.result().await.unwrap_err().code,
        crate::ErrorCode::Cancelled
    );
}

#[tokio::test]
async fn direct_read_batch_round_trips_through_bip() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ComplexAck};
    use bacnet_network::layer::NetworkLayer;
    use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
    use bacnet_transport::bip::BipTransport;
    use bacnet_npdu::NetworkPriority;
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bytes::{Bytes, BytesMut};
    use std::net::Ipv4Addr;

    let mut peer = NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    let mut incoming = peer.start().await.unwrap();
    let peer_mac = peer.local_mac().to_vec();
    let responder = tokio::spawn(async move {
        let received = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
            .await
            .expect("runtime did not send the read")
            .expect("peer receive channel closed");
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(received.apdu).unwrap() else {
            panic!("expected a confirmed request")
        };
        let property = ReadPropertyRequest::decode(&request.service_request).unwrap();
        let ack = ReadPropertyACK {
            object_identifier: property.object_identifier,
            property_identifier: property.property_identifier,
            property_array_index: property.property_array_index,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        };
        let mut service_ack = BytesMut::new();
        ack.encode(&mut service_ack);
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from(service_ack.to_vec()),
            }),
        )
        .unwrap();
        peer.send_apdu(
            &encoded,
            &received.source_mac,
            false,
            NetworkPriority::Normal,
        )
        .await
        .unwrap();
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
                input_index: 7,
                attachment_id: AttachmentId::from(1),
                target: PropertyTarget::new(
                    peer_mac,
                    ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_INPUT, 1)
                        .unwrap(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                ),
            }],
            timeout: Duration::from_secs(2),
        })
        .await
        .unwrap()
        .result()
        .await
        .unwrap();
    assert_eq!(outcomes[0].input_index, 7);
    assert_eq!(
        outcomes[0].raw_value.as_deref(),
        Some(&[0x44, 0x42, 0x90, 0x00, 0x00][..])
    );
    assert!(outcomes[0].error.is_none());

    responder.await.unwrap();
    runtime.stop().await.unwrap();
}
