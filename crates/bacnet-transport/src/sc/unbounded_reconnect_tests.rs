use super::*;
use crate::port::{TransportHealthState, TransportPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

async fn accept(hub: &LoopbackWebSocket, hub_vmac: Vmac) {
    let request = hub.recv().await.unwrap();
    let request = decode_sc_message(&request).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let response = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: request.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &response);
    hub.send(&wire).await.unwrap();
}

async fn start(transport: &mut ScTransport<LoopbackWebSocket>, hub: &LoopbackWebSocket) {
    let (started, ()) = tokio::join!(transport.start(), accept(hub, [0x10; 6]));
    let _rx = started.unwrap();
}

async fn spin_until(mut predicate: impl FnMut() -> bool) {
    for _ in 0..100 {
        if predicate() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("condition did not become true");
}

#[tokio::test(start_paused = true)]
async fn retry_forever_alternates_caps_backoff_and_recovers_beyond_bounded_budget() {
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let started_at = tokio::time::Instant::now();
    let calls = Arc::new(StdMutex::new(Vec::<(&'static str, u64)>::new()));
    let primary_count = Arc::new(AtomicUsize::new(0));
    let failover_count = Arc::new(AtomicUsize::new(0));
    let (success_tx, mut success_rx) = mpsc::unbounded_channel();

    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(10, 20))
        .with_connector({
            let calls = calls.clone();
            let primary_count = primary_count.clone();
            move || {
                calls
                    .lock()
                    .unwrap()
                    .push(("primary", started_at.elapsed().as_millis() as u64));
                primary_count.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("scripted primary failure".into())) }
            }
        })
        .with_failover_connector({
            let calls = calls.clone();
            let failover_count = failover_count.clone();
            move || {
                calls
                    .lock()
                    .unwrap()
                    .push(("failover", started_at.elapsed().as_millis() as u64));
                let call = failover_count.fetch_add(1, Ordering::SeqCst) + 1;
                let success_tx = success_tx.clone();
                async move {
                    if call == 1 {
                        Err(Error::Encoding("scripted failover failure".into()))
                    } else {
                        let (client, hub) = LoopbackWebSocket::pair();
                        success_tx.send(hub).unwrap();
                        Ok(client)
                    }
                }
            }
        });

    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    drop(initial_hub);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;

    for _ in 0..3 {
        tokio::time::advance(Duration::from_millis(
            if primary_count.load(Ordering::SeqCst) == 0 {
                10
            } else {
                20
            },
        ))
        .await;
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(20)).await;
    spin_until(|| success_rx.len() == 1).await;
    let success_hub = success_rx.try_recv().unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("primary", 10),
            ("failover", 30),
            ("primary", 50),
            ("failover", 70),
        ]
    );
    accept(&success_hub, [0x20; 6]).await;
    spin_until(|| health.borrow().state == TransportHealthState::Up).await;
    let snapshot = health.borrow().clone();
    assert_eq!(snapshot.active_hub.as_deref(), Some("failover"));
    assert_eq!(snapshot.attempt, 4);
    assert!(snapshot.since.is_some());
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn retry_forever_single_hub_does_not_schedule_fictitious_failover_attempts() {
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let started_at = tokio::time::Instant::now();
    let calls = Arc::new(StdMutex::new(Vec::<u64>::new()));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(10, 20))
        .with_connector({
            let calls = calls.clone();
            move || {
                calls
                    .lock()
                    .unwrap()
                    .push(started_at.elapsed().as_millis() as u64);
                async { Err(Error::Encoding("scripted primary failure".into())) }
            }
        });

    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    drop(initial_hub);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;

    for delay in [10, 20, 20, 20] {
        tokio::time::advance(Duration::from_millis(delay)).await;
        tokio::task::yield_now().await;
    }
    spin_until(|| calls.lock().unwrap().len() >= 4).await;
    assert_eq!(&calls.lock().unwrap()[..4], &[10, 30, 50, 70]);
    let snapshot = health.borrow().clone();
    assert_eq!(snapshot.active_hub.as_deref(), Some("primary"));
    assert!(snapshot.attempt >= 4);
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn peer_disconnect_ack_is_sent_before_recovery_while_old_socket_stays_open() {
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let (hub_tx, mut hub_rx) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(10, 10))
        .with_connector(move || {
            let (client, hub) = LoopbackWebSocket::pair();
            hub_tx.send(hub).unwrap();
            async move { Ok(client) }
        });

    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    initial_hub.send(&[8, 0, 0x12, 0x34]).await.unwrap();
    assert_eq!(initial_hub.recv().await.unwrap(), [9, 0, 0x12, 0x34]);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;
    assert!(health
        .borrow()
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("DisconnectRequest")));

    tokio::time::advance(Duration::from_millis(10)).await;
    spin_until(|| hub_rx.len() == 1).await;
    let replacement_hub = hub_rx.try_recv().unwrap();
    accept(&replacement_hub, [0x20; 6]).await;
    spin_until(|| health.borrow().state == TransportHealthState::Up).await;
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn failover_dial_error_is_preserved_in_health() {
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let failover_calls = Arc::new(AtomicUsize::new(0));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(10, 20))
        .with_connector(|| async { Err(Error::Encoding("primary unavailable".into())) })
        .with_failover_connector({
            let failover_calls = failover_calls.clone();
            move || {
                failover_calls.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("failover certificate mismatch".into())) }
            }
        });

    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    drop(initial_hub);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;
    tokio::time::advance(Duration::from_millis(10)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    spin_until(|| failover_calls.load(Ordering::SeqCst) == 1).await;
    let snapshot = health.borrow().clone();
    assert!(snapshot
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("failover certificate mismatch")));
    transport.stop().await.unwrap();
}

fn fail_random48() -> Result<Vmac, Error> {
    Err(Error::Encoding("scripted Random-48 failure".into()))
}

#[tokio::test(start_paused = true)]
async fn retry_forbidden_connect_nak_terminates_unbounded_recovery_as_failed() {
    let _guard = set_test_random48_vmac_generator(fail_random48);
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let (hub_tx, mut hub_rx) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(10, 20))
        .with_connector(move || {
            let (client, hub) = LoopbackWebSocket::pair();
            hub_tx.send(hub).unwrap();
            async move { Ok(client) }
        });
    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    drop(initial_hub);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;
    tokio::time::advance(Duration::from_millis(10)).await;
    spin_until(|| hub_rx.len() == 1).await;
    let hub = hub_rx.try_recv().unwrap();
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    let mut payload = vec![ScFunction::ConnectRequest.to_raw(), 1, 0, 0, 0, 0, 0];
    payload[3..5].copy_from_slice(&ErrorClass::COMMUNICATION.to_raw().to_be_bytes());
    payload[5..7].copy_from_slice(&ErrorCode::NODE_DUPLICATE_VMAC.to_raw().to_be_bytes());
    let nak = ScMessage {
        function: ScFunction::Result,
        message_id: request.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &nak);
    hub.send(&wire).await.unwrap();
    spin_until(|| health.borrow().state == TransportHealthState::Failed).await;
    let snapshot = health.borrow().clone();
    assert!(snapshot
        .detail
        .as_deref()
        .is_some_and(|value| !value.is_empty()));
    assert!(snapshot
        .last_error
        .as_deref()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(snapshot.attempt, 1);
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_cancels_unbounded_recovery_during_backoff() {
    let (client, initial_hub) = LoopbackWebSocket::pair();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(5_000, 10_000)
        .with_reconnect(ScReconnectConfig::unbounded(60_000, 60_000))
        .with_connector({
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(Error::Encoding("unexpected dial".into())) }
            }
        });
    start(&mut transport, &initial_hub).await;
    let health = transport.transport_health_changes();
    drop(initial_hub);
    spin_until(|| health.borrow().state == TransportHealthState::Reconnecting).await;
    transport.stop().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(health.borrow().state, TransportHealthState::Down);
}
