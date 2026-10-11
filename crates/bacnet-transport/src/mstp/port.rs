use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_mstp_codec::Frame;
use bacnet_mstp_core::{
    Action, Direction, MasterSnapshot, NetworkPriority, ReplyDecision, RequestId, TransmitFailure,
    TransmitId,
};
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tracing::warn;

use crate::mstp_frame::{
    decode_frame_stream_with_scratch, find_preamble, retain_lone_preamble_byte, MstpFrame,
    StreamDecode, BROADCAST_MAC, MAX_EXTENDED_MPDU_DATA,
};
use crate::port::{ReceivedNpdu, TransportPort};

pub(super) use super::core_adapter::CoreAdapter;
use super::{
    calculate_host_stale_partial_timeout_us, MasterNode, MasterState, MstpConfig, SerialDirection,
    SerialPort, MSTP_MAX_FRAME_BUF,
};

/// Assemble host serial reads and return only complete, CRC-validated frames.
fn assemble_host_chunk(frame_buf: &mut Vec<u8>, chunk: &[u8]) -> Vec<MstpFrame> {
    let mut scratch = vec![0u8; MAX_EXTENDED_MPDU_DATA];
    assemble_host_chunk_with_scratch(frame_buf, chunk, &mut scratch)
}

/// Assemble host serial reads using caller-owned decoded extended-payload
/// storage. The encoded receive buffer and transformed COBS payload workspace
/// are independently bounded.
fn assemble_host_chunk_with_scratch(
    frame_buf: &mut Vec<u8>,
    chunk: &[u8],
    scratch: &mut [u8],
) -> Vec<MstpFrame> {
    let mut frames = Vec::new();
    let mut remaining = chunk;
    while !remaining.is_empty() {
        let available = MSTP_MAX_FRAME_BUF.saturating_sub(frame_buf.len());
        if available == 0 {
            warn!("MS/TP: full incomplete host assembly, discarding one byte");
            frame_buf.drain(..1);
            continue;
        }
        let take = available.min(remaining.len());
        frame_buf.extend_from_slice(&remaining[..take]);
        remaining = &remaining[take..];
        loop {
            let preamble_pos = match find_preamble(frame_buf) {
                Some(pos) => pos,
                None => {
                    retain_lone_preamble_byte(frame_buf);
                    break;
                }
            };
            if preamble_pos > 0 {
                frame_buf.drain(..preamble_pos);
            }
            match decode_frame_stream_with_scratch(frame_buf, scratch) {
                StreamDecode::Complete { frame, consumed } => {
                    frame_buf.drain(..consumed);
                    frames.push(frame);
                }
                StreamDecode::NeedMore => break,
                StreamDecode::Invalid { discard } => {
                    frame_buf.drain(..discard.min(frame_buf.len()).max(1));
                }
            }
        }
    }
    debug_assert!(frame_buf.len() <= MSTP_MAX_FRAME_BUF);
    frames
}

fn mirror_legacy_enqueue(legacy: &mut MasterNode, destination: u8, npdu: Bytes) {
    if legacy.tx_queue.len() >= super::MAX_TX_QUEUE_DEPTH {
        legacy.tx_queue.pop_front();
    }
    legacy.tx_queue.push_back((destination, npdu));
}

fn mirror_legacy_transmit(legacy: &mut MasterNode, frame: &[u8]) {
    let Ok((sent, _)) = bacnet_mstp_codec::decode_frame(frame) else {
        return;
    };
    if !matches!(
        sent.header.frame_type,
        bacnet_mstp_codec::FrameType::BACnetDataExpectingReply
            | bacnet_mstp_codec::FrameType::BACnetDataNotExpectingReply
    ) {
        return;
    }
    if let Some(index) = legacy.tx_queue.iter().position(|(destination, npdu)| {
        *destination == sent.header.destination && npdu.as_ref() == sent.data
    }) {
        legacy.tx_queue.remove(index);
    }
}

struct Runtime<S: SerialPort> {
    core: Arc<Mutex<CoreAdapter>>,
    legacy: Arc<Mutex<MasterNode>>,
    serial: Arc<S>,
    write_lock: Mutex<()>,
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
    wake: Notify,
    transmit_direction_ready: AtomicBool,
    receive_faulted: AtomicBool,
    reply_tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
}

#[derive(Debug)]
enum OwnedAction {
    Transmit {
        id: TransmitId,
        frame: Vec<u8>,
    },
    Deliver {
        source: u8,
        destination: u8,
        npdu: Bytes,
        request_id: Option<RequestId>,
    },
    Direction(Direction),
    Idle,
}

async fn poll_owned_action<S: SerialPort>(runtime: &Runtime<S>) -> OwnedAction {
    let mut core = runtime.core.lock().await;
    match core.core.poll_action() {
        Action::Transmit { id, frame } => OwnedAction::Transmit {
            id,
            frame: frame.to_vec(),
        },
        Action::DeliverNpdu {
            source,
            destination,
            npdu,
            request_id,
        } => OwnedAction::Deliver {
            source,
            destination,
            npdu: Bytes::copy_from_slice(npdu),
            request_id,
        },
        Action::SetDirection(direction) => OwnedAction::Direction(direction),
        Action::Idle => OwnedAction::Idle,
    }
}

async fn restore_receive<S: SerialPort>(serial: &S) -> Result<(), Error> {
    let mut last_error = None;
    for _ in 0..3 {
        match tokio::time::timeout(
            tokio::time::Duration::from_millis(100),
            serial.set_direction(SerialDirection::Receive),
        )
        .await
        {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                last_error = Some(Error::Encoding(
                    "MS/TP receive-direction watchdog expired".into(),
                ));
            }
        }
    }
    Err(last_error.expect("receive restoration always attempts at least once"))
}

async fn drive_actions<S: SerialPort>(runtime: Arc<Runtime<S>>) {
    loop {
        match poll_owned_action(&runtime).await {
            OwnedAction::Idle => break,
            OwnedAction::Direction(direction) => {
                let serial_direction = match direction {
                    Direction::Receive => SerialDirection::Receive,
                    Direction::Transmit => SerialDirection::Transmit,
                };
                let timeout = {
                    let core = runtime.core.lock().await;
                    core.hard_deadline().unwrap_or_else(|| {
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(1)
                    })
                };
                let result = if direction == Direction::Receive {
                    restore_receive(runtime.serial.as_ref()).await
                } else {
                    tokio::time::timeout_at(timeout, runtime.serial.set_direction(serial_direction))
                        .await
                        .unwrap_or_else(|_| {
                            Err(Error::Encoding(
                                "MS/TP direction transition watchdog expired".into(),
                            ))
                        })
                };
                runtime.transmit_direction_ready.store(
                    direction == Direction::Transmit && result.is_ok(),
                    Ordering::Release,
                );
                if let Err(error) = result {
                    if direction == Direction::Receive {
                        runtime.receive_faulted.store(true, Ordering::Release);
                    }
                    warn!("MS/TP direction change failed: {error}");
                }
            }
            OwnedAction::Transmit { id, frame } => {
                if !runtime
                    .transmit_direction_ready
                    .swap(false, Ordering::AcqRel)
                {
                    let mut core = runtime.core.lock().await;
                    let now = core.now();
                    if let Err(error) = core.core.on_transmit_aborted(id, TransmitFailure::Io, now)
                    {
                        warn!("MS/TP direction-failed transmit abort rejected: {error:?}");
                    }
                    let mut legacy = runtime.legacy.lock().await;
                    core.sync_legacy(&mut legacy);
                    continue;
                }
                let started = {
                    let mut core = runtime.core.lock().await;
                    let now = core.now();
                    core.core.on_transmit_started(id, now)
                };
                if let Err(error) = started {
                    warn!("MS/TP transmit start rejected: {error:?}");
                    continue;
                }
                let watchdog = {
                    let core = runtime.core.lock().await;
                    core.hard_deadline().unwrap_or_else(|| {
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(1)
                    })
                };
                let _write_guard = runtime.write_lock.lock().await;
                let result = tokio::time::timeout_at(watchdog, async {
                    runtime.serial.write(&frame).await?;
                    runtime.serial.drain().await
                })
                .await
                .unwrap_or_else(|_| Err(Error::Encoding("MS/TP transmit watchdog expired".into())));
                drop(_write_guard);
                let mut core = runtime.core.lock().await;
                let now = core.now();
                let completion = match result {
                    Ok(()) => core.core.on_transmit_complete(id, now),
                    Err(_) => core.core.on_transmit_aborted(id, TransmitFailure::Io, now),
                };
                if let Err(error) = completion {
                    warn!("MS/TP transmit completion rejected: {error:?}");
                }
                let mut legacy = runtime.legacy.lock().await;
                mirror_legacy_transmit(&mut legacy, &frame);
                core.sync_legacy(&mut legacy);
            }
            OwnedAction::Deliver {
                source,
                destination,
                npdu,
                request_id,
            } => {
                let (reply_tx, reply_rx) = if request_id.is_some() {
                    let (tx, rx) = oneshot::channel();
                    (Some(tx), Some(rx))
                } else {
                    (None, None)
                };
                let received = ReceivedNpdu {
                    npdu: npdu.clone(),
                    source_mac: bacnet_types::MacAddr::from_slice(&[source]),
                    link_layer_group: destination == BROADCAST_MAC,
                    data_attributes: Vec::new(),
                    transport_meta: None,
                    reply_tx,
                };
                let _ = runtime.npdu_tx.try_send(received);
                if let (Some(id), Some(reply_rx)) = (request_id, reply_rx) {
                    let deadline = {
                        let core = runtime.core.lock().await;
                        core.hard_deadline().unwrap_or_else(|| {
                            tokio::time::Instant::now() + tokio::time::Duration::from_millis(250)
                        })
                    };
                    let reply_runtime = runtime.clone();
                    let task = tokio::spawn(async move {
                        let reply = match tokio::time::timeout_at(deadline, reply_rx).await {
                            Ok(reply) => reply,
                            Err(_) => return,
                        };
                        let mut core = reply_runtime.core.lock().await;
                        let now = core.now();
                        let result = match reply {
                            Ok(data) => core.core.complete_request(
                                id,
                                ReplyDecision::Data(data.as_ref()),
                                now,
                            ),
                            Err(_) => core
                                .core
                                .complete_request(id, ReplyDecision::Cancelled, now),
                        };
                        if let Err(error) = result {
                            if matches!(error, bacnet_mstp_core::CoreError::ReplyOversize { .. }) {
                                let abandoned =
                                    core.core
                                        .complete_request(id, ReplyDecision::Abandoned, now);
                                if let Err(abandon_error) = abandoned {
                                    warn!("MS/TP oversized reply abandonment failed: {abandon_error:?}");
                                }
                            } else {
                                warn!("MS/TP application reply rejected: {error:?}");
                            }
                        }
                        let mut legacy = reply_runtime.legacy.lock().await;
                        core.sync_legacy(&mut legacy);
                        drop(legacy);
                        drop(core);
                        reply_runtime.wake.notify_one();
                    });
                    let mut tasks = runtime
                        .reply_tasks
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    tasks.retain(|task| !task.is_finished());
                    tasks.push(task.abort_handle());
                }
            }
        }
    }
}

/// MS/TP transport implementing [`TransportPort`].
pub struct MstpTransport<S: SerialPort> {
    serial: Option<S>,
    config: MstpConfig,
    local_mac: [u8; 1],
    node: Option<Arc<Mutex<MasterNode>>>,
    core: Option<Arc<Mutex<CoreAdapter>>>,
    runtime: Option<Arc<Runtime<S>>>,
    recv_task: Option<tokio::task::JoinHandle<()>>,
}

impl<S: SerialPort> MstpTransport<S> {
    pub fn new(serial: S, config: MstpConfig) -> Self {
        let mac = config.this_station;
        Self {
            serial: Some(serial),
            config,
            local_mac: [mac],
            node: None,
            core: None,
            runtime: None,
            recv_task: None,
        }
    }

    /// Get the deprecated compatibility node state for existing callers.
    #[deprecated(note = "use snapshot() and counters() on MstpTransport instead")]
    pub fn node_state(&self) -> Option<&Arc<Mutex<MasterNode>>> {
        self.node.as_ref()
    }

    /// Return a copyable snapshot of the platform-neutral state machine.
    pub async fn snapshot(&self) -> Option<MasterSnapshot> {
        let core = self.core.as_ref()?.lock().await;
        Some(core.core.snapshot())
    }

    /// Return protocol and queue counters from the platform-neutral core.
    pub async fn counters(&self) -> Option<bacnet_mstp_core::MstpCounters> {
        let core = self.core.as_ref()?.lock().await;
        Some(core.core.counters())
    }

    fn release_resources(&mut self) {
        if let Some(task) = self.recv_task.take() {
            task.abort();
        }
        if let Some(runtime) = self.runtime.take() {
            for task in runtime
                .reply_tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .drain(..)
            {
                task.abort();
            }
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(error) = restore_receive(runtime.serial.as_ref()).await {
                        warn!("MS/TP abort could not restore receive direction: {error}");
                    }
                });
            }
        }
        self.serial.take();
        self.node.take();
        self.core.take();
    }
}

impl<S: SerialPort> TransportPort for MstpTransport<S> {
    fn transport_kind(&self) -> &'static str {
        "mstp"
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        const NPDU_CHANNEL_CAPACITY: usize = 64;
        let (npdu_tx, npdu_rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let origin = tokio::time::Instant::now();
        let core = Arc::new(Mutex::new(CoreAdapter::new(&self.config, origin)?));
        let legacy = Arc::new(Mutex::new(MasterNode::new(self.config.clone())?));
        let serial = Arc::new(
            self.serial
                .take()
                .ok_or_else(|| Error::Encoding("MS/TP transport already started".into()))?,
        );
        let runtime = Arc::new(Runtime {
            core: core.clone(),
            legacy: legacy.clone(),
            serial: serial.clone(),
            write_lock: Mutex::new(()),
            npdu_tx,
            wake: Notify::new(),
            transmit_direction_ready: AtomicBool::new(false),
            receive_faulted: AtomicBool::new(false),
            reply_tasks: std::sync::Mutex::new(Vec::new()),
        });
        let host_stale_timeout = calculate_host_stale_partial_timeout_us(self.config.baud_rate);
        let task_runtime = runtime.clone();
        let task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            let mut frame_buf = Vec::with_capacity(MSTP_MAX_FRAME_BUF);
            let mut decode_scratch = vec![0u8; MAX_EXTENDED_MPDU_DATA];
            let mut last_byte_time = None;
            loop {
                if task_runtime.receive_faulted.swap(false, Ordering::AcqRel) {
                    if let Err(error) = restore_receive(task_runtime.serial.as_ref()).await {
                        warn!("MS/TP receive-direction recovery failed: {error}");
                        task_runtime.receive_faulted.store(true, Ordering::Release);
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        continue;
                    }
                }
                let deadline = {
                    let core = task_runtime.core.lock().await;
                    core.deadline().unwrap_or_else(|| {
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(3600)
                    })
                };
                tokio::select! {
                    result = task_runtime.serial.read(&mut recv_buf) => {
                        let count = match result {
                            Ok(count) => count,
                            Err(error) => { warn!("MS/TP serial read error: {error}"); break; }
                        };
                        if count == 0 { continue; }
                        let now = tokio::time::Instant::now();
                        if !frame_buf.is_empty()
                            && last_byte_time.is_some_and(|last| {
                                now.duration_since(last).as_micros() as u64 > host_stale_timeout
                            })
                        {
                            frame_buf.clear();
                        }
                        last_byte_time = Some(now);
                        {
                            let mut core = task_runtime.core.lock().await;
                            let timestamp = core.now();
                            if let Err(error) = core.core.on_host_activity(count, timestamp) {
                                warn!("MS/TP host activity rejected by core: {error:?}");
                            }
                        }
                        for frame in assemble_host_chunk_with_scratch(
                            &mut frame_buf,
                            &recv_buf[..count],
                            &mut decode_scratch,
                        ) {
                            // The hosted state machine and its action storage
                            // remain standard-frame sized until X3. Decode and
                            // resynchronize extended frames here without
                            // passing them into a standard-only core.
                            if frame.frame_type.is_cobs() {
                                warn!(
                                    frame_type = frame.frame_type.to_raw(),
                                    "MS/TP extended frame requires an extended core profile"
                                );
                                continue;
                            }
                            let mut core = task_runtime.core.lock().await;
                            let timestamp = core.now();
                            let validated = Frame::new(
                                frame.frame_type,
                                frame.destination,
                                frame.source,
                                &frame.data,
                            );
                            if let Err(error) = core.core.on_validated_frame(validated, timestamp) {
                                warn!("MS/TP frame rejected by core: {error:?}");
                            }
                            let mut legacy = task_runtime.legacy.lock().await;
                            core.sync_legacy(&mut legacy);
                        }
                        drive_actions(task_runtime.clone()).await;
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        let mut core = task_runtime.core.lock().await;
                        if let Err(error) = core.service_deadline() {
                            warn!("MS/TP deadline rejected by core: {error:?}");
                        }
                        let mut legacy = task_runtime.legacy.lock().await;
                        core.sync_legacy(&mut legacy);
                        drop(legacy);
                        drop(core);
                        drive_actions(task_runtime.clone()).await;
                    }
                    _ = task_runtime.wake.notified() => {
                        drive_actions(task_runtime.clone()).await;
                    }
                }
            }
        });
        self.node = Some(legacy);
        self.core = Some(core);
        self.runtime = Some(runtime);
        self.recv_task = Some(task);
        Ok(npdu_rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(runtime) = self.runtime.take() {
            for task in runtime
                .reply_tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .drain(..)
            {
                task.abort();
            }
            restore_receive(runtime.serial.as_ref()).await?;
        }
        if let Some(node) = &self.node {
            let mut node = node.lock().await;
            node.tx_queue.clear();
            node.state = MasterState::Idle;
            node.expected_reply_source = None;
        }
        self.core.take();
        Ok(())
    }

    fn abort(&mut self) {
        self.release_resources();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        if mac.len() != 1 {
            return Err(Error::Encoding(format!(
                "MS/TP MAC must be 1 byte, got {}",
                mac.len()
            )));
        }
        let core = self.core.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MS/TP transport not started",
            ))
        })?;
        let mut core = core.lock().await;
        let now = core.now();
        core.core
            .enqueue_npdu(mac[0], npdu, NetworkPriority::Normal, now)
            .map_err(|error| Error::Encoding(format!("MS/TP queue rejected NPDU: {error:?}")))?;
        let mut legacy = self
            .node
            .as_ref()
            .expect("core and legacy start together")
            .lock()
            .await;
        mirror_legacy_enqueue(&mut legacy, mac[0], Bytes::copy_from_slice(npdu));
        core.sync_legacy(&mut legacy);
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[BROADCAST_MAC]).await
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
    fn max_apdu_length(&self) -> u16 {
        480
    }
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        mac == [BROADCAST_MAC]
    }
}

impl<S: SerialPort> Drop for MstpTransport<S> {
    fn drop(&mut self) {
        self.release_resources();
    }
}

/// In-memory loopback serial port for unit testing.
pub struct LoopbackSerial {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tx: mpsc::Sender<Vec<u8>>,
    leftover: Mutex<Vec<u8>>,
}

impl LoopbackSerial {
    pub fn pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        (
            Self {
                rx: Mutex::new(rx_a),
                tx: tx_a,
                leftover: Mutex::new(Vec::new()),
            },
            Self {
                rx: Mutex::new(rx_b),
                tx: tx_b,
                leftover: Mutex::new(Vec::new()),
            },
        )
    }
}

impl SerialPort for LoopbackSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::Encoding("loopback write failed".into()))
    }
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut leftover = self.leftover.lock().await;
        if !leftover.is_empty() {
            let len = leftover.len().min(buf.len());
            buf[..len].copy_from_slice(&leftover[..len]);
            leftover.drain(..len);
            return Ok(len);
        }
        drop(leftover);
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(data) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                if data.len() > buf.len() {
                    self.leftover
                        .lock()
                        .await
                        .extend_from_slice(&data[buf.len()..]);
                }
                Ok(len)
            }
            None => Err(Error::Encoding("loopback channel closed".into())),
        }
    }
}

/// A serial port implementation that always errors.
pub struct NoSerial;

impl SerialPort for NoSerial {
    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }
    async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }
}

#[cfg(test)]
#[path = "port_assembly_tests.rs"]
mod assembly_tests;
