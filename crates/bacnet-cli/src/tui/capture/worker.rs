//! The capture pumps: read a source, summarise each frame, and offer them
//! to the UI in batches.
//!
//! Nothing here waits for the UI. Frames go out when a batch is full or
//! every [`REPORT_EVERY`], whichever is first, so capture shares the UI's
//! event channel with a few events a second. A batch the channel can't
//! take is dropped and its frames counted, beside what the source itself
//! lost (a tap queue or a monitor subscription that filled up), so while a
//! source runs the Capture screen's dropped counter is exact: every frame
//! the source offered is in the ring or in that count. Counts the UI
//! hasn't heard of ride on the next batch that gets through. No lock is
//! held across an await.

use std::any::Any;
use std::time::{Duration, SystemTime};

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::mstp::codec::Frame;
use bacnet_transport::mstp::monitor::{
    LineErrorKind, LineEvent, MonitorFrames, MonitorRecord, MstpMonitor,
};
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::{FrameTap, FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bytes::Bytes;
use tokio::sync::mpsc;

use super::{Batch, CaptureCommand, CaptureEvent, CaptureFrame, Direction, FrameSummary, Framing};
use crate::core::target::AddressStyle;
use crate::decode::frame::summarize_frame;
use crate::tui::message::WorkerEvent;
use crate::tui::monitor::lines::CliSerial;
use crate::tui::worker::{EventSink, Offer};

/// Most frames in one batch.
pub(crate) const BATCH: usize = 256;

/// How often counts the UI hasn't heard of are sent on their own.
pub(crate) const REPORT_EVERY: Duration = Duration::from_millis(250);

/// The tap queue: frames the pump may fall behind by.
pub(crate) const TAP_QUEUE: usize = 4_096;

/// Where frames come from.
pub(crate) enum Source {
    /// A transport's frame tap.
    Tap {
        /// The subscription.
        rx: FrameTapReceiver,
        /// The transport's link.
        framing: Framing,
        /// This node's address as the device table writes it.
        local_text: String,
    },
    /// The MS/TP monitor's live stream.
    Monitor {
        /// The subscription.
        frames: MonitorFrames,
        /// Wall-clock time of the monitor's start, which records count from.
        epoch: SystemTime,
    },
    /// Frames a test hands in, and its own count of those it lost.
    #[cfg(test)]
    Test {
        /// The frames.
        rx: mpsc::Receiver<CaptureFrame>,
        /// Lost before reaching `rx`.
        lost: std::sync::Arc<std::sync::atomic::AtomicU64>,
    },
}

/// What a source gave. Held only until the frame joins a batch, so its
/// size is not worth a box per frame.
#[allow(clippy::large_enum_variant)]
enum Next {
    /// A frame.
    Frame(CaptureFrame),
    /// The end, and why.
    End(String),
}

impl Source {
    /// The next frame, waiting for one. Cancel safe.
    async fn next(&mut self) -> Next {
        let next = match self {
            Self::Tap {
                rx,
                framing,
                local_text,
            } => (rx.recv().await).map(|frame| convert(frame, *framing, local_text)),
            Self::Monitor { frames, epoch } => {
                let epoch = *epoch;
                frames.recv().await.map(|record| from_record(record, epoch))
            }
            #[cfg(test)]
            Self::Test { rx, .. } => rx.recv().await,
        };
        next.map_or_else(|| Next::End("the source closed".into()), Next::Frame)
    }

    /// The next frame if one is waiting.
    fn try_next(&mut self) -> Option<CaptureFrame> {
        match self {
            Self::Tap {
                rx,
                framing,
                local_text,
            } => rx
                .try_recv()
                .map(|frame| convert(frame, *framing, local_text)),
            Self::Monitor { frames, epoch } => {
                let epoch = *epoch;
                frames.try_recv().map(|record| from_record(record, epoch))
            }
            #[cfg(test)]
            Self::Test { rx, .. } => rx.try_recv().ok(),
        }
    }

    /// Frames the source lost so far.
    fn lost(&self) -> u64 {
        match self {
            Self::Tap { rx, .. } => rx.dropped(),
            Self::Monitor { frames, .. } => frames.dropped(),
            #[cfg(test)]
            Self::Test { lost, .. } => lost.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// A tapped frame, summarised, with its endpoints.
fn convert(frame: TappedFrame, framing: Framing, local_text: &str) -> CaptureFrame {
    let summary = summarize_frame(framing, &frame.bytes);
    let inbound = frame.direction == TapDirection::Inbound;
    let peer_text = match frame.peer {
        TapPeer::Udp(peer) => peer.to_string(),
        TapPeer::ScHub(hub) => format!("hub {}", crate::tui::sc::hub_name(hub)),
        TapPeer::Mstp(station) => AddressStyle::Mstp.format(&[station]),
        _ => "?".to_string(),
    };
    let (mut source, mut destination) = if inbound {
        (peer_text, local_text.to_string())
    } else {
        (local_text.to_string(), peer_text)
    };
    // The link's own addresses, where the frame carries them, are the
    // device table's (VMACs, MS/TP stations); B/IP has only UDP's.
    if framing != Framing::Bip {
        source = summary.link_source.clone().unwrap_or(source);
        destination = summary.link_destination.clone().unwrap_or(destination);
    }
    let (source, destination, via) = route(source, destination, &summary, !inbound);
    CaptureFrame {
        time: frame.timestamp,
        framing,
        direction: if inbound {
            Direction::In
        } else {
            Direction::Out
        },
        source,
        destination,
        via,
        bytes: frame.bytes,
        summary,
    }
}

/// A routed NPDU's ends as routed stations (`2:0x05`), with the link peer
/// it went through: the router it came from, or the one it was sent to.
pub(crate) fn route(
    source: String,
    destination: String,
    summary: &FrameSummary,
    outbound: bool,
) -> (String, String, Option<String>) {
    if summary.routed_source.is_none() && summary.routed_destination.is_none() {
        return (source, destination, None);
    }
    let via = if outbound {
        destination.clone()
    } else {
        source.clone()
    };
    (
        summary.routed_source.clone().unwrap_or(source),
        summary.routed_destination.clone().unwrap_or(destination),
        Some(via),
    )
}

/// A monitor record as a frame: a valid frame encoded again from its
/// header and data, a line error as a frame with no octets.
fn from_record(record: MonitorRecord, epoch: SystemTime) -> CaptureFrame {
    let time = epoch + record.at;
    let line = |link: &str, problem: String, destination: Option<u8>| {
        let summary = FrameSummary {
            link: link.into(),
            problem: Some(problem),
            link_destination: destination.map(|d| AddressStyle::Mstp.format(&[d])),
            ..FrameSummary::default()
        };
        (Bytes::new(), summary)
    };
    let (bytes, summary) = match record.event {
        LineEvent::Frame { header, .. } => {
            let mut wire = vec![0; 16 + record.data.len() + record.data.len() / 128];
            let frame = Frame::new(
                header.frame_type,
                header.destination,
                header.source,
                &record.data,
            );
            match frame.encode_into(&mut wire) {
                Ok(length) => {
                    wire.truncate(length);
                    let summary = summarize_frame(Framing::Mstp, &wire);
                    (Bytes::from(wire), summary)
                }
                Err(error) => line("frame", format!("not encodable: {error}"), None),
            }
        }
        LineEvent::Error {
            kind,
            destination,
            octets,
        } => line(
            "line-error",
            format!(
                "{}, {octets} octets dropped",
                match kind {
                    LineErrorKind::HeaderCrc => "header-crc",
                    LineErrorKind::DataCrc => "data-crc",
                    _ => "malformed",
                }
            ),
            destination,
        ),
        LineEvent::Noise { octets } => line("noise", format!("{octets} octets of noise"), None),
        LineEvent::FrameAbort => line("frame-abort", "a frame stopped midway".into(), None),
        _ => line("line-event", "unknown line event".into(), None),
    };
    let (source, destination, via) = route(
        summary.link_source.clone().unwrap_or_default(),
        summary.link_destination.clone().unwrap_or_default(),
        &summary,
        false,
    );
    CaptureFrame {
        time,
        framing: Framing::Mstp,
        direction: Direction::Wire,
        source,
        destination,
        via,
        bytes,
        summary,
    }
}

/// The frame tap of a transport the CLI builds, and its link.
pub(crate) fn tap_of<T: TransportPort + 'static>(transport: &T) -> Option<(FrameTap, Framing)> {
    let any = transport as &dyn Any;
    if let Some(bip) = any.downcast_ref::<BipTransport>() {
        return Some((bip.frame_tap().clone(), Framing::Bip));
    }
    if let Some(bip6) = any.downcast_ref::<Bip6Transport>() {
        return Some((bip6.frame_tap().clone(), Framing::Bip6));
    }
    #[cfg(feature = "sc-tls")]
    {
        use bacnet_transport::sc::ScTransport;
        use bacnet_transport::sc_tls::TlsWebSocket;
        if let Some(sc) = any.downcast_ref::<ScTransport<TlsWebSocket>>() {
            return Some((sc.frame_tap().clone(), Framing::Sc));
        }
    }
    let mstp = any.downcast_ref::<MstpTransport<CliSerial>>()?;
    Some((mstp.frame_tap().clone(), Framing::Mstp))
}

/// A session's capture: wait for [`CaptureCommand::Start`], subscribe to
/// the client's frame tap, and pump until the session drops `commands`.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<CaptureCommand>,
    style: AddressStyle,
) {
    while let Some(CaptureCommand::Start) = commands.recv().await {
        let Some((tap, framing)) = tap_of(client.transport()) else {
            let ended = CaptureEvent::Ended("this transport has no frame tap".into());
            sink.deliver(WorkerEvent::Capture(ended)).await;
            continue;
        };
        let local_text = style.format(client.local_mac());
        let source = Source::Tap {
            rx: tap.subscribe_with_capacity(TAP_QUEUE),
            framing,
            local_text: local_text.clone(),
        };
        let started = format!("{} tap at {local_text}", framing.label());
        tracing::info!("capture: {started}");
        if sink
            .deliver(WorkerEvent::Capture(CaptureEvent::Started {
                what: started,
                session: true,
            }))
            .await
        {
            pump(source, sink, &mut commands).await;
        }
        return;
    }
}

/// The monitor session's capture: as [`run`], on the monitor's stream.
pub(crate) async fn run_monitor(
    monitor: &MstpMonitor,
    sink: &EventSink,
    mut commands: mpsc::Receiver<CaptureCommand>,
    started: tokio::time::Instant,
) {
    if let Some(CaptureCommand::Start) = commands.recv().await {
        let epoch = SystemTime::now()
            .checked_sub(started.elapsed())
            .unwrap_or_else(SystemTime::now);
        let source = Source::Monitor {
            frames: monitor.frames(),
            epoch,
        };
        let started = CaptureEvent::Started {
            what: "MS/TP monitor, every frame on the line".into(),
            session: true,
        };
        if sink.deliver(WorkerEvent::Capture(started)).await {
            pump(source, sink, &mut commands).await;
        }
    }
}

/// Pump `source` to the UI until the session ends (`commands` closes, or
/// the sink is cancelled) or the source does.
pub(crate) async fn pump(
    mut source: Source,
    sink: &EventSink,
    commands: &mut mpsc::Receiver<CaptureCommand>,
) {
    let mut counts = Counts::default();
    let mut batch = Vec::new();
    let mut report = tokio::time::interval(REPORT_EVERY);
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => return,
            received = commands.recv() => {
                // Another Start is a no-op: this capture already runs.
                if received.is_none() {
                    return;
                }
            }
            next = source.next() => match next {
                Next::Frame(frame) => {
                    batch.push(frame);
                    while batch.len() < BATCH {
                        match source.try_next() {
                            Some(frame) => batch.push(frame),
                            None => break,
                        }
                    }
                    // A full batch goes now; a partial one waits for the
                    // tick, so a busy source costs the shared channel a
                    // few events a second, not one per wake-up.
                    if batch.len() >= BATCH && !counts.offer(&mut batch, &source, sink) {
                        return;
                    }
                }
                Next::End(reason) => {
                    counts.finish(batch, &source, sink).await;
                    tracing::info!("capture ended: {reason}");
                    sink.deliver(WorkerEvent::Capture(CaptureEvent::Ended(reason))).await;
                    return;
                }
            },
            _ = report.tick() => {
                if !counts.offer(&mut batch, &source, sink) {
                    return;
                }
            }
        }
    }
}

/// What the UI has been told about losses.
#[derive(Default)]
struct Counts {
    /// Losses the UI has counted.
    reported: u64,
    /// Frames in batches the event channel refused.
    refused: u64,
}

impl Counts {
    /// Offer `batch`, with the losses not yet reported; false once the UI
    /// has gone.
    fn offer(&mut self, batch: &mut Vec<CaptureFrame>, source: &Source, sink: &EventSink) -> bool {
        let lost = source.lost() + self.refused;
        if batch.is_empty() && lost == self.reported {
            return true;
        }
        let frames = std::mem::take(batch);
        let count = frames.len() as u64;
        let event = CaptureEvent::Frames(Box::new(Batch {
            frames,
            lost: lost - self.reported,
        }));
        // Refused frames count in the screen's own dropped figure only.
        match sink.offer_uncounted(WorkerEvent::Capture(event)) {
            Offer::Sent => {
                self.reported = lost;
                true
            }
            Offer::Dropped => {
                self.refused += count;
                true
            }
            Offer::Closed => false,
        }
    }

    /// The last batch and counts, when the source has ended: waited for,
    /// so the UI's count is exact to the end.
    async fn finish(&mut self, frames: Vec<CaptureFrame>, source: &Source, sink: &EventSink) {
        let lost = source.lost() + self.refused;
        if frames.is_empty() && lost == self.reported {
            return;
        }
        let batch = Box::new(Batch {
            frames,
            lost: lost - self.reported,
        });
        if sink
            .deliver(WorkerEvent::Capture(CaptureEvent::Frames(batch)))
            .await
        {
            self.reported = lost;
        }
    }
}
