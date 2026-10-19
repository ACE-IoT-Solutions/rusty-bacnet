//! Hub-connector events and link status (#959).
//!
//! The transport tests run on tokio's paused clock (#1549): the heartbeat,
//! backoff and restore timers and the tests' waits all read tokio's clock,
//! so each step is exact and a loaded runner cannot reorder them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

use super::super::data_attribute_tests::hub_accept;
use super::super::test_waits::until;
use super::super::{
    LoopbackWebSocket, ScConnectError, ScConnectionState, ScReconnectConfig, ScTransport,
    ScWebSocketErrorKind, WebSocketPort,
};
use super::*;
use crate::port::TransportPort;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage, Vmac};

const PRIMARY: Vmac = [0x10; 6];
const FAILOVER: Vmac = [0x20; 6];

async fn next(events: &mut broadcast::Receiver<ScEvent>) -> ScEvent {
    timeout(Duration::from_secs(30), events.recv())
        .await
        .expect("no SC event in time")
        .expect("SC event stream ended or lagged")
}

async fn send(ws: &LoopbackWebSocket, msg: &ScMessage) {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, msg);
    ws.send(&buf).await.unwrap();
}

fn message(function: ScFunction, message_id: u16, payload: Vec<u8>) -> ScMessage {
    ScMessage {
        function,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    }
}

/// A BVLC-Result NAK answering `result_for`, with a Communication class.
fn nak(message_id: u16, result_for: ScFunction, code: ErrorCode) -> ScMessage {
    let mut payload = vec![result_for.to_raw(), 0x01, 0x00];
    payload.extend_from_slice(&ErrorClass::COMMUNICATION.to_raw().to_be_bytes());
    payload.extend_from_slice(&code.to_raw().to_be_bytes());
    message(ScFunction::Result, message_id, payload)
}

fn connected(hub: ScActiveHub, hub_vmac: Vmac) -> ScEvent {
    ScEvent::Connected { hub, hub_vmac }
}

#[test]
fn a_lagging_subscriber_loses_the_oldest_events_and_never_blocks_the_sender() {
    let observer = ScObserver::new();
    let mut slow = observer.subscribe();
    let overflow = 10;
    // Synchronous sends: a full receiver cannot make the connector wait.
    for attempt in 0..SC_EVENT_CAPACITY as u32 + overflow {
        observer.reconnect_attempt(ScActiveHub::Primary, attempt, Duration::ZERO);
    }
    assert_eq!(
        slow.try_recv(),
        Err(TryRecvError::Lagged(u64::from(overflow)))
    );
    assert_eq!(
        slow.try_recv(),
        Ok(ScEvent::ReconnectAttempt {
            hub: ScActiveHub::Primary,
            attempt: overflow,
            backoff: Duration::ZERO,
        })
    );
}

#[test]
fn errors_map_to_their_disconnect_causes() {
    let websocket = |kind| {
        ScConnectError::WebSocket {
            kind,
            message: "fixture".into(),
            #[cfg(feature = "sc-tls")]
            untrusted_peer: None,
        }
        .into_bacnet_error()
    };
    let nak = ScConnectError::HandshakeNak {
        result_for: ScFunction::ConnectRequest,
        error_header_marker: 0,
        error_class: ErrorClass::COMMUNICATION.to_raw(),
        error_code: ErrorCode::NOT_A_BACNET_SC_HUB.to_raw(),
        error_details: String::new(),
        duplicate_vmac_reseeded: false,
    }
    .into_bacnet_error();
    for (error, cause) in [
        (
            websocket(ScWebSocketErrorKind::TlsHandshake),
            ScDisconnectCause::WebSocket(ScWebSocketErrorKind::TlsHandshake),
        ),
        (
            websocket(ScWebSocketErrorKind::TcpDial),
            ScDisconnectCause::WebSocket(ScWebSocketErrorKind::TcpDial),
        ),
        (
            websocket(ScWebSocketErrorKind::Closed),
            ScDisconnectCause::PeerClose,
        ),
        (
            websocket(ScWebSocketErrorKind::StreamEnded),
            ScDisconnectCause::PeerClose,
        ),
        (
            Error::Timeout(Duration::from_secs(1)),
            ScDisconnectCause::ConnectTimeout,
        ),
        (
            nak,
            ScDisconnectCause::Nak {
                result_for: ScFunction::ConnectRequest,
                error_class: ErrorClass::COMMUNICATION,
                error_code: ErrorCode::NOT_A_BACNET_SC_HUB,
            },
        ),
        (
            ScConnectError::ConnectAcceptMismatch.into_bacnet_error(),
            ScDisconnectCause::UnexpectedConnectReply,
        ),
        (
            Error::Encoding("plain".into()),
            ScDisconnectCause::Other("encoding error: plain".into()),
        ),
    ] {
        let mapped = ScDisconnectCause::from_error(&error);
        if let ScDisconnectCause::Other(_) = cause {
            assert!(matches!(mapped, ScDisconnectCause::Other(ref text) if text.contains("plain")));
        } else {
            assert_eq!(mapped, cause, "{error}");
        }
    }
}

/// Heartbeat timing and the heartbeat-timeout cause on one connection.
#[tokio::test(start_paused = true)]
async fn heartbeat_timeout_is_reported_with_its_cause_after_age_and_rtt() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(100, 300);
    let mut events = transport.connection_events();
    let before = transport.connection_status();
    assert_eq!(before.state, ScConnectionState::Disconnected);
    assert_eq!(before.active_hub, None);
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );

    tokio::time::advance(Duration::from_millis(50)).await;
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Primary));
    assert_eq!(status.hub_vmac, Some(PRIMARY));
    assert_eq!(status.heartbeat_age, Some(Duration::from_millis(50)));
    assert_eq!(status.heartbeat_rtt, None);

    // The hub answers the first Heartbeat-Request 20 ms late.
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::HeartbeatRequest);
    tokio::time::advance(Duration::from_millis(20)).await;
    send(
        &hub,
        &message(ScFunction::HeartbeatAck, request.message_id, Vec::new()),
    )
    .await;
    until("heartbeat RTT published", || async {
        transport.connection_status().heartbeat_rtt.is_some()
    })
    .await;
    let status = transport.connection_status();
    assert_eq!(status.heartbeat_rtt, Some(Duration::from_millis(20)));
    assert_eq!(status.heartbeat_age, Some(Duration::ZERO));

    // Silent from now on, the link times out on the heartbeat timer.
    assert_eq!(
        next(&mut events).await,
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::HeartbeatTimeout,
        }
    );
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Disconnected);
    assert_eq!(status.hub_vmac, None);
    assert_eq!(status.heartbeat_age, None);
    transport.stop().await.unwrap();
    // Already reported: stopping adds no second Disconnected.
    assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
    drop(hub);
}

/// A Connect-Request NAK and a NAK that ends a live connection both carry
/// the error class and code.
#[tokio::test(start_paused = true)]
async fn naks_report_their_error_class_and_code() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6]).with_device_uuid([1; 16]);
    let mut events = transport.connection_events();
    let refuse = async {
        let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
        let code = ErrorCode::NOT_A_BACNET_SC_HUB;
        send(
            &hub,
            &nak(request.message_id, ScFunction::ConnectRequest, code),
        )
        .await;
    };
    let (result, ()) = tokio::join!(transport.start(), refuse);
    assert!(result.is_err());
    assert_eq!(
        next(&mut events).await,
        ScEvent::ConnectFailed {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::Nak {
                result_for: ScFunction::ConnectRequest,
                error_class: ErrorClass::COMMUNICATION,
                error_code: ErrorCode::NOT_A_BACNET_SC_HUB,
            },
        }
    );

    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6]).with_device_uuid([1; 16]);
    let mut events = transport.connection_events();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );
    let code = ErrorCode::BVLC_FUNCTION_UNKNOWN;
    send(&hub, &nak(9, ScFunction::HeartbeatRequest, code)).await;
    assert_eq!(
        next(&mut events).await,
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::Nak {
                result_for: ScFunction::HeartbeatRequest,
                error_class: ErrorClass::COMMUNICATION,
                error_code: code,
            },
        }
    );
    transport.stop().await.unwrap();
}

type Dial =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<LoopbackWebSocket, Error>> + Send>>;

/// A connector handing each dial's hub end to `hubs`, refusing while `up` is false.
fn connector(
    up: Arc<AtomicBool>,
    hubs: mpsc::UnboundedSender<LoopbackWebSocket>,
) -> impl Fn() -> Dial + Send + Sync + 'static {
    move || {
        let up = up.load(Ordering::SeqCst);
        let hubs = hubs.clone();
        Box::pin(async move {
            if !up {
                return Err(Error::Encoding("fixture hub is down".into()));
            }
            let (client, hub) = LoopbackWebSocket::pair();
            hubs.send(hub)
                .map_err(|_| Error::Encoding("fixture hub receiver dropped".into()))?;
            Ok(client)
        })
    }
}

/// The hub ends the primary connection: the connector fails over, then
/// restores the primary, and every step is an event in order (Annex AB.5.2).
#[tokio::test(start_paused = true)]
async fn failover_and_primary_restore_are_events_in_order() {
    let (client, primary_hub) = LoopbackWebSocket::pair();
    let primary_up = Arc::new(AtomicBool::new(false));
    let (primary_tx, mut primary_dials) = mpsc::unbounded_channel();
    let (failover_tx, mut failover_dials) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_connector(connector(primary_up.clone(), primary_tx))
        .with_failover_connector(connector(Arc::new(AtomicBool::new(true)), failover_tx));
    let mut events = transport.connection_events();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&primary_hub, PRIMARY));
    let _rx = rx.unwrap();
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );

    send(
        &primary_hub,
        &message(ScFunction::DisconnectRequest, 0x4242, Vec::new()),
    )
    .await;
    assert_eq!(
        next(&mut events).await,
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::PeerClose,
        }
    );
    // The transport answers with a Disconnect-ACK; then the hub closes.
    let ack = decode_sc_message(&primary_hub.recv().await.unwrap()).unwrap();
    assert_eq!(ack.function, ScFunction::DisconnectAck);
    drop(primary_hub);

    assert_eq!(
        next(&mut events).await,
        ScEvent::ReconnectAttempt {
            hub: ScActiveHub::Primary,
            attempt: 1,
            backoff: Duration::from_millis(25),
        }
    );
    assert!(matches!(
        next(&mut events).await,
        ScEvent::ConnectFailed {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::Other(ref text),
        } if text.contains("fixture hub is down")
    ));
    assert_eq!(next(&mut events).await, ScEvent::FailoverEngaged);
    let failover_hub = failover_dials.recv().await.unwrap();
    hub_accept(&failover_hub, FAILOVER).await;
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Failover, FAILOVER)
    );
    let status = transport.connection_status();
    assert_eq!(status.active_hub, Some(ScActiveHub::Failover));
    assert_eq!(status.hub_vmac, Some(FAILOVER));

    // The primary comes back; the next restore attempt reaches it.
    primary_up.store(true, Ordering::SeqCst);
    let primary_hub = primary_dials.recv().await.unwrap();
    hub_accept(&primary_hub, PRIMARY).await;
    assert_eq!(next(&mut events).await, ScEvent::PrimaryRestored);
    assert_eq!(
        next(&mut events).await,
        connected(ScActiveHub::Primary, PRIMARY)
    );
    let status = transport.connection_status();
    assert_eq!(status.state, ScConnectionState::Connected);
    assert_eq!(status.active_hub, Some(ScActiveHub::Primary));
    assert_eq!(status.hub_vmac, Some(PRIMARY));
    // The replaced failover connection gets a Disconnect-Request.
    let farewell = decode_sc_message(&failover_hub.recv().await.unwrap()).unwrap();
    assert_eq!(farewell.function, ScFunction::DisconnectRequest);

    transport.stop().await.unwrap();
    assert_eq!(
        next(&mut events).await,
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause: ScDisconnectCause::LocalClose,
        }
    );
    assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
    drop(primary_hub);
}
