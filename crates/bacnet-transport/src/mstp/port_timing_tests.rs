use bacnet_mstp_core::{Action, Direction, Instant as CoreInstant};
use bytes::{Bytes, BytesMut};
use std::sync::{Arc, Mutex as StdMutex};

use super::port::CoreAdapter;
use super::*;
use crate::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use crate::port::TransportPort;

fn expecting_reply_frame() -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x04, 0x10]),
    }
}

fn timing_config() -> MstpConfig {
    MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    }
}

struct DirectionRecordingSerial {
    inner: LoopbackSerial,
    events: Arc<StdMutex<Vec<&'static str>>>,
}

struct StallingWriteSerial {
    inner: LoopbackSerial,
    events: Arc<StdMutex<Vec<&'static str>>>,
}

struct FailingReceiveSerial {
    inner: LoopbackSerial,
    events: Arc<StdMutex<Vec<&'static str>>>,
    fail_receive_once: Arc<StdMutex<bool>>,
}

struct HardwareBusySerial {
    inner: LoopbackSerial,
    events: Arc<StdMutex<Vec<&'static str>>>,
    transmitter_empty: Arc<tokio::sync::Notify>,
}

struct StallingDirectionSerial {
    inner: LoopbackSerial,
    writes: Arc<StdMutex<usize>>,
}

struct DelayedDirectionSerial {
    inner: LoopbackSerial,
    transmit_started: Arc<StdMutex<bool>>,
    writes: Arc<StdMutex<usize>>,
}

impl SerialPort for DelayedDirectionSerial {
    async fn set_direction(&self, direction: SerialDirection) -> Result<(), Error> {
        if direction == SerialDirection::Transmit {
            *self.transmit_started.lock().unwrap() = true;
            tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
        }
        Ok(())
    }

    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }
}

impl SerialPort for StallingDirectionSerial {
    async fn set_direction(&self, _direction: SerialDirection) -> Result<(), Error> {
        std::future::pending().await
    }

    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }
}

impl SerialPort for StallingWriteSerial {
    async fn set_direction(&self, direction: SerialDirection) -> Result<(), Error> {
        self.events.lock().unwrap().push(match direction {
            SerialDirection::Transmit => "tx",
            SerialDirection::Receive => "rx",
        });
        Ok(())
    }

    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        std::future::pending().await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }
}

impl SerialPort for FailingReceiveSerial {
    async fn set_direction(&self, direction: SerialDirection) -> Result<(), Error> {
        if direction == SerialDirection::Receive {
            let mut fail = self.fail_receive_once.lock().unwrap();
            if *fail {
                *fail = false;
                self.events.lock().unwrap().push("rx-fail");
                return Err(Error::Encoding(
                    "injected receive transition failure".into(),
                ));
            }
        }
        self.events.lock().unwrap().push(match direction {
            SerialDirection::Transmit => "tx",
            SerialDirection::Receive => "rx",
        });
        Ok(())
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.events.lock().unwrap().push("write");
        self.inner.write(data).await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }

    async fn drain(&self) -> Result<(), Error> {
        self.events.lock().unwrap().push("drain");
        Ok(())
    }
}

impl SerialPort for HardwareBusySerial {
    async fn set_direction(&self, direction: SerialDirection) -> Result<(), Error> {
        self.events.lock().unwrap().push(match direction {
            SerialDirection::Transmit => "tx",
            SerialDirection::Receive => "rx",
        });
        Ok(())
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.events.lock().unwrap().push("write");
        self.inner.write(data).await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }

    async fn drain(&self) -> Result<(), Error> {
        self.events.lock().unwrap().push("hardware-busy");
        self.transmitter_empty.notified().await;
        self.events.lock().unwrap().push("hardware-empty");
        Ok(())
    }
}

impl SerialPort for DirectionRecordingSerial {
    async fn set_direction(&self, direction: SerialDirection) -> Result<(), Error> {
        self.events.lock().unwrap().push(match direction {
            SerialDirection::Transmit => "tx",
            SerialDirection::Receive => "rx",
        });
        Ok(())
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.events.lock().unwrap().push("write");
        self.inner.write(data).await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }

    async fn drain(&self) -> Result<(), Error> {
        self.events.lock().unwrap().push("drain");
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn ready_data_reply_does_not_wait_for_reply_delay() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    received
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::BACnetDataNotExpectingReply);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(50));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn core_direction_actions_bracket_write_and_physical_drain() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let events = Arc::new(StdMutex::new(Vec::new()));
    let serial_transport = DirectionRecordingSerial {
        inner: serial_inner,
        events: events.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();

    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    let mut response_buf = [0u8; 64];
    serial_peer.read(&mut response_buf).await.unwrap();
    tokio::task::yield_now().await;

    assert_eq!(&*events.lock().unwrap(), &["tx", "write", "drain", "rx"]);
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn receive_transition_waits_for_hardware_transmitter_empty() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let events = Arc::new(StdMutex::new(Vec::new()));
    let transmitter_empty = Arc::new(tokio::sync::Notify::new());
    let serial_transport = HardwareBusySerial {
        inner: serial_inner,
        events: events.clone(),
        transmitter_empty: transmitter_empty.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(10)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        if events.lock().unwrap().contains(&"hardware-busy") {
            break;
        }
    }
    assert_eq!(&*events.lock().unwrap(), &["tx", "write", "hardware-busy"]);

    transmitter_empty.notify_one();
    for _ in 0..20 {
        tokio::task::yield_now().await;
        if events.lock().unwrap().last() == Some(&"rx") {
            break;
        }
    }
    assert_eq!(
        &*events.lock().unwrap(),
        &["tx", "write", "hardware-busy", "hardware-empty", "rx"]
    );
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn late_reply_scheduler_wake_still_precedes_hard_wire_expiry() {
    let origin = tokio::time::Instant::now();
    let mut adapter = CoreAdapter::new(&timing_config(), origin).unwrap();
    adapter
        .core
        .on_validated_frame(
            bacnet_mstp_codec::Frame::new(
                bacnet_mstp_codec::FrameType::BACnetDataExpectingReply,
                3,
                7,
                &[0x01, 0x04, 0x10],
            ),
            CoreInstant::ZERO,
        )
        .unwrap();
    assert!(matches!(
        adapter.core.poll_action(),
        Action::DeliverNpdu {
            request_id: Some(_),
            ..
        }
    ));
    tokio::time::advance(tokio::time::Duration::from_millis(226)).await;

    adapter.service_deadline().unwrap();
    let turnaround = adapter.core.next_deadline().unwrap();
    adapter.core.on_deadline(turnaround).unwrap();

    assert_eq!(
        adapter.core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    assert!(matches!(
        adapter.core.poll_action(),
        Action::Transmit { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn stalled_serial_write_is_aborted_by_the_transmit_watchdog() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let events = Arc::new(StdMutex::new(Vec::new()));
    let serial_transport = StallingWriteSerial {
        inner: serial_inner,
        events: events.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(10)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        if !events.lock().unwrap().is_empty() {
            break;
        }
    }
    assert_eq!(&*events.lock().unwrap(), &["tx"]);

    tokio::time::advance(tokio::time::Duration::from_secs(2)).await;
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }

    assert_eq!(&*events.lock().unwrap(), &["tx", "rx"]);
    assert_eq!(
        transport.snapshot().await.unwrap().state,
        bacnet_mstp_core::MasterState::Idle
    );
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_during_stalled_write_restores_receive_direction() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let events = Arc::new(StdMutex::new(Vec::new()));
    let serial_transport = StallingWriteSerial {
        inner: serial_inner,
        events: events.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(10)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        if events.lock().unwrap().as_slice() == ["tx"] {
            break;
        }
    }

    transport.stop().await.unwrap();
    assert_eq!(&*events.lock().unwrap(), &["tx", "rx"]);
}

#[tokio::test(start_paused = true)]
async fn transient_receive_direction_failure_is_retried() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let events = Arc::new(StdMutex::new(Vec::new()));
    let serial_transport = FailingReceiveSerial {
        inner: serial_inner,
        events: events.clone(),
        fail_receive_once: Arc::new(StdMutex::new(true)),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    let mut response = [0; 64];
    serial_peer.read(&mut response).await.unwrap();
    tokio::task::yield_now().await;

    assert_eq!(
        &*events.lock().unwrap(),
        &["tx", "write", "drain", "rx-fail", "rx"]
    );
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stalled_direction_change_is_bounded_and_prevents_the_write() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let writes = Arc::new(StdMutex::new(0));
    let serial_transport = StallingDirectionSerial {
        inner: serial_inner,
        writes: writes.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    npdu_rx
        .recv()
        .await
        .unwrap()
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(&[0x01, 0x00, 0x30, 0x01]))
        .unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(10)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(tokio::time::Duration::from_secs(2)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    assert_eq!(*writes.lock().unwrap(), 0);
    assert_eq!(
        transport.snapshot().await.unwrap().state,
        bacnet_mstp_core::MasterState::Idle
    );
    assert!(transport.stop().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn direction_delay_crossing_reply_expiry_suppresses_the_write() {
    let (serial_inner, serial_peer) = LoopbackSerial::pair();
    let transmit_started = Arc::new(StdMutex::new(false));
    let writes = Arc::new(StdMutex::new(0));
    let serial_transport = DelayedDirectionSerial {
        inner: serial_inner,
        transmit_started: transmit_started.clone(),
        writes: writes.clone(),
    };
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    serial_peer.write(&encoded).await.unwrap();
    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();

    tokio::time::advance(tokio::time::Duration::from_millis(225)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if *transmit_started.lock().unwrap() {
            break;
        }
    }
    assert!(*transmit_started.lock().unwrap());
    tokio::time::advance(tokio::time::Duration::from_millis(30)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    assert_eq!(*writes.lock().unwrap(), 0);
    assert_eq!(
        transport.snapshot().await.unwrap().state,
        bacnet_mstp_core::MasterState::Idle
    );
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reply_postponed_starts_before_reply_delay_limit() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut encoded = BytesMut::new();
    encode_frame(&mut encoded, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&encoded).await.unwrap();

    // Keep the application sender alive without replying so the deadline,
    // rather than channel cancellation, selects ReplyPostponed.
    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();

    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn second_data_request_does_not_extend_first_reply_deadline() {
    let (serial_transport, serial_peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial_transport, timing_config());
    let mut npdu_rx = transport.start().await.unwrap();
    let mut first = BytesMut::new();
    encode_frame(&mut first, &expecting_reply_frame()).unwrap();
    let started = tokio::time::Instant::now();
    serial_peer.write(&first).await.unwrap();

    let received = npdu_rx.recv().await.unwrap();
    let _reply_tx = received.reply_tx.unwrap();
    tokio::time::advance(tokio::time::Duration::from_millis(100)).await;

    let mut second = BytesMut::new();
    let second_request = MstpFrame {
        source: 8,
        data: Bytes::from_static(&[0x01, 0x04, 0x20]),
        ..expecting_reply_frame()
    };
    encode_frame(&mut second, &second_request).unwrap();
    serial_peer.write(&second).await.unwrap();
    tokio::task::yield_now().await;
    assert!(npdu_rx.try_recv().is_err());

    let mut response_buf = [0u8; 64];
    let response_len = serial_peer.read(&mut response_buf).await.unwrap();
    let (response, _) = decode_frame(&response_buf[..response_len]).unwrap();
    assert_eq!(response.frame_type, FrameType::ReplyPostponed);
    assert_eq!(response.destination, 7);
    assert!(started.elapsed() < tokio::time::Duration::from_millis(T_REPLY_DELAY_MS));
    transport.stop().await.unwrap();
}
