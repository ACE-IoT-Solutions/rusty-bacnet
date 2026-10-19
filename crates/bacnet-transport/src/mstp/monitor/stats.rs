//! The monitor's statistics engine: pure bookkeeping over timestamped line
//! events, with no port, clock or task, so it can be fed from a serial
//! port, a capture replay or a test script alike.

use std::collections::BTreeMap;
use std::time::Duration;

use bacnet_mstp_codec::{FrameError, FrameHeader, FrameType, BROADCAST_MAC};

/// How long the line may stay silent before a master takes the token as
/// lost: T_no_token, 500 ms (Clause 9.5.3). A silence at least this long
/// after earlier traffic counts as one lost-token event.
pub const NO_TOKEN_SILENCE: Duration = Duration::from_millis(500);

/// Upper bounds (exclusive) of the [`SilenceHistogram`] buckets; a last
/// bucket holds every silence of 500 ms or more.
///
/// The bounds include the Clause 9.5.3 timers a reader compares silences
/// with: T_usage_delay (15 ms), T_usage_timeout (20 ms, up to 35 ms on a
/// host), T_reply_delay (250 ms) and T_no_token (500 ms).
pub const SILENCE_BUCKET_BOUNDS: [Duration; 10] = [
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(5),
    Duration::from_millis(10),
    Duration::from_millis(15),
    Duration::from_millis(20),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    NO_TOKEN_SILENCE,
];

/// The time one octet takes on the line at `baud_rate`: ten bit times, a
/// start bit, eight data bits and a stop bit (Clause 9.2.3). Zero for a
/// zero rate.
#[must_use]
pub fn octet_time(baud_rate: u32) -> Duration {
    if baud_rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos(10_000_000_000 / u64::from(baud_rate))
}

/// One thing the monitor saw on the line, for [`MonitorStats::record`].
///
/// The timestamp that goes with it is when its last octet arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineEvent {
    /// A frame that passed every check.
    Frame {
        /// Its header.
        header: FrameHeader,
        /// Octets on the wire, preamble and CRCs included.
        wire_length: usize,
    },
    /// A frame dropped by the decoder: a bad header CRC, a bad data CRC or
    /// CRC-32K, or any other check (see [`LineErrorKind`]).
    Error {
        /// What was wrong.
        kind: LineErrorKind,
        /// The destination, when the header CRC passed.
        destination: Option<u8>,
        /// Octets the decoder dropped for it.
        octets: usize,
    },
    /// Octets that never began a frame, dropped while looking for a
    /// preamble.
    Noise {
        /// How many.
        octets: usize,
    },
    /// A partial frame given up after the line went quiet in the middle of
    /// it for longer than the monitor's frame-abort time.
    FrameAbort,
}

/// Why the decoder dropped a candidate frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LineErrorKind {
    /// The header CRC failed.
    HeaderCrc,
    /// The data CRC (or an extended frame's CRC-32K) failed.
    DataCrc,
    /// Any other check failed: a header naming source 255, a Length out of
    /// range, a frame longer than the decoder's storage. Counted as a
    /// framing error.
    Framing,
}

impl LineErrorKind {
    /// Classify a decoder error.
    #[must_use]
    pub fn from_frame_error(error: FrameError) -> Self {
        match error {
            FrameError::HeaderCrcMismatch => Self::HeaderCrc,
            FrameError::DataCrcMismatch { .. } => Self::DataCrc,
            _ => Self::Framing,
        }
    }
}

/// Frame counts by type (Clause 9.3). Types without a field of their own,
/// reserved and proprietary ones, add up in `other`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameTypeCounts {
    /// Token.
    pub token: u64,
    /// Poll For Master.
    pub poll_for_master: u64,
    /// Reply To Poll For Master.
    pub reply_to_poll_for_master: u64,
    /// Test_Request.
    pub test_request: u64,
    /// Test_Response.
    pub test_response: u64,
    /// BACnet Data Expecting Reply.
    pub data_expecting_reply: u64,
    /// BACnet Data Not Expecting Reply.
    pub data_not_expecting_reply: u64,
    /// Reply Postponed.
    pub reply_postponed: u64,
    /// BACnet Extended Data Expecting Reply (decoded only).
    pub extended_data_expecting_reply: u64,
    /// BACnet Extended Data Not Expecting Reply (decoded only).
    pub extended_data_not_expecting_reply: u64,
    /// Every other type: reserved (8-31, 34-127) and proprietary (128-255).
    pub other: u64,
}

impl FrameTypeCounts {
    fn slot(&mut self, frame_type: FrameType) -> &mut u64 {
        match frame_type {
            FrameType::TOKEN => &mut self.token,
            FrameType::POLL_FOR_MASTER => &mut self.poll_for_master,
            FrameType::REPLY_TO_POLL_FOR_MASTER => &mut self.reply_to_poll_for_master,
            FrameType::TEST_REQUEST => &mut self.test_request,
            FrameType::TEST_RESPONSE => &mut self.test_response,
            FrameType::BACNET_DATA_EXPECTING_REPLY => &mut self.data_expecting_reply,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY => &mut self.data_not_expecting_reply,
            FrameType::REPLY_POSTPONED => &mut self.reply_postponed,
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY => {
                &mut self.extended_data_expecting_reply
            }
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY => {
                &mut self.extended_data_not_expecting_reply
            }
            _ => &mut self.other,
        }
    }

    fn add(&mut self, frame_type: FrameType) {
        let slot = self.slot(frame_type);
        *slot = slot.saturating_add(1);
    }

    /// The count for `frame_type`; a type without a field of its own reads
    /// `other`.
    #[must_use]
    pub fn get(&self, frame_type: FrameType) -> u64 {
        let mut copy = *self;
        *copy.slot(frame_type)
    }

    /// Every frame counted.
    #[must_use]
    pub fn total(&self) -> u64 {
        [
            self.token,
            self.poll_for_master,
            self.reply_to_poll_for_master,
            self.test_request,
            self.test_response,
            self.data_expecting_reply,
            self.data_not_expecting_reply,
            self.reply_postponed,
            self.extended_data_expecting_reply,
            self.extended_data_not_expecting_reply,
            self.other,
        ]
        .iter()
        .fold(0u64, |sum, count| sum.saturating_add(*count))
    }
}

/// Count, minimum, maximum, total and latest of a series of intervals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DurationStats {
    /// Samples taken.
    pub count: u64,
    /// The shortest, once there is a sample.
    pub min: Option<Duration>,
    /// The longest, once there is a sample.
    pub max: Option<Duration>,
    /// The most recent, once there is a sample.
    pub last: Option<Duration>,
    /// All samples added up (saturating).
    pub total: Duration,
}

impl DurationStats {
    fn add(&mut self, sample: Duration) {
        self.count = self.count.saturating_add(1);
        self.min = Some(self.min.map_or(sample, |min| min.min(sample)));
        self.max = Some(self.max.map_or(sample, |max| max.max(sample)));
        self.last = Some(sample);
        self.total = self.total.saturating_add(sample);
    }

    /// The mean, once there is a sample.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        let count = u32::try_from(self.count).ok().filter(|count| *count > 0)?;
        Some(self.total / count)
    }
}

/// What one station did on the line.
///
/// A station gets an entry when it sends a valid frame or one is addressed
/// to it; [`is_present`](Self::is_present) tells the two apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StationStats {
    /// Valid frames this station sent, by type.
    pub frames_sent: FrameTypeCounts,
    /// Valid frames addressed to this station (broadcasts not included), by
    /// type.
    pub frames_received: FrameTypeCounts,
    /// When it last sent a valid frame.
    pub last_seen: Option<Duration>,
    /// Token frames addressed to it.
    pub tokens_received: u64,
    /// Token frames it sent.
    pub tokens_passed: u64,
    /// Token frames it sent again to the same station with nothing heard in
    /// between: its successor did not start using the token in time.
    pub token_retries: u64,
    /// The station it last passed the token to.
    pub next_station: Option<u8>,
    /// The highest station it sent a Poll For Master to.
    ///
    /// A master polls only up to its own Max_Master (Clause 9.5.6), so this
    /// is the monitor's inferred Max_Master for it: a lower bound until the
    /// master has polled through its whole range, which it does a step at a
    /// time as it holds the token.
    pub max_master_inferred: Option<u8>,
    /// Time between successive Token frames it sent: one rotation of the
    /// ring, seen from this station.
    pub token_rotation: DurationStats,
    /// Time from a Token addressed to it to the start of its next frame,
    /// when that frame came straight after the Token: how quickly it took
    /// up the token.
    pub token_usage_delay: DurationStats,
}

impl StationStats {
    /// Whether the station sent at least one valid frame: it is on the line,
    /// as opposed to only addressed.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.frames_sent.total() > 0
    }

    /// Whether the station passed the token on or polled for masters, which
    /// only a master that holds the token does.
    #[must_use]
    pub fn is_master(&self) -> bool {
        self.frames_sent.token > 0 || self.frames_sent.poll_for_master > 0
    }
}

/// Counts of what the decoder rejected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LineErrors {
    /// Headers whose CRC failed.
    pub header_crc: u64,
    /// Frames whose data CRC (or CRC-32K) failed.
    pub data_crc: u64,
    /// Frames rejected by any other check ([`LineErrorKind::Framing`]).
    pub framing: u64,
    /// Partial frames given up after the line went quiet in the middle.
    pub frame_aborts: u64,
    /// Octets dropped as noise between frames.
    pub noise_octets: u64,
}

/// A silence on the line long enough for the masters to take the token as
/// lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LostToken {
    /// When the line became active again.
    pub at: Duration,
    /// How long it had been silent.
    pub silence: Duration,
    /// The station whose frame ended the silence, normally the master
    /// that generated a new token (Clause 9.5.6.7), when that was a valid
    /// frame.
    pub regenerated_by: Option<u8>,
}

/// Silences before each piece of line activity, bucketed by
/// [`SILENCE_BUCKET_BOUNDS`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SilenceHistogram {
    /// `counts[i]` holds silences below `SILENCE_BUCKET_BOUNDS[i]` and at
    /// or above the bound before it; the last holds the rest.
    pub counts: [u64; SILENCE_BUCKET_BOUNDS.len() + 1],
    /// The longest silence seen.
    pub longest: Option<Duration>,
}

impl SilenceHistogram {
    fn add(&mut self, silence: Duration) {
        let bucket = SILENCE_BUCKET_BOUNDS
            .iter()
            .position(|bound| silence < *bound)
            .unwrap_or(SILENCE_BUCKET_BOUNDS.len());
        self.counts[bucket] = self.counts[bucket].saturating_add(1);
        self.longest = Some(self.longest.map_or(silence, |longest| longest.max(silence)));
    }
}

/// How precise the timestamps are.
///
/// The monitor stamps each read from the port with the host's clock and
/// works back from the read's end to each frame's at the baud rate. A USB
/// adapter holds octets back before the host sees them (16 ms by default
/// on FTDI parts) and hands them over in batches, so every time the
/// statistics hold, and every interval worked out from them, is uncertain
/// by about `resolution`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimingQuality {
    /// The uncertainty the monitor was configured with: what the adapter
    /// is known or assumed to add.
    pub resolution: Duration,
    /// One octet's time at the configured baud rate.
    pub octet_time: Duration,
    /// Reads from the port that returned octets.
    pub reads: u64,
    /// The most octets one read returned. Many octets per read, on a slow
    /// line, means a coarse adapter.
    pub max_octets_per_read: u64,
}

/// What the monitor has seen so far.
///
/// Times are measured from when the monitor started. Every count
/// saturates at `u64::MAX`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MonitorStats {
    /// Valid frames by type, every station together.
    pub frames: FrameTypeCounts,
    /// Octets received.
    pub octets: u64,
    /// Valid frames whose data was COBS-encoded (Clause 9.10). The monitor
    /// decodes them and checks their CRC-32K, nothing more.
    pub cobs_frames: u64,
    /// What the decoder rejected.
    pub errors: LineErrors,
    /// Per-station statistics, by address.
    pub stations: BTreeMap<u8, StationStats>,
    /// Token frames to a single station.
    pub token_passes: u64,
    /// Silences of at least [`NO_TOKEN_SILENCE`] after earlier traffic.
    pub lost_tokens: u64,
    /// The latest of them.
    pub last_lost_token: Option<LostToken>,
    /// Silences before each piece of line activity.
    pub silences: SilenceHistogram,
    /// How precise the times are.
    pub timing: TimingQuality,
    /// When the line was last active.
    pub last_activity: Option<Duration>,
    /// The last valid frame, if nothing else came after it: when it ended,
    /// and its header.
    last_frame: Option<(Duration, FrameHeader)>,
    /// When each station last sent a Token, for its rotation time.
    last_token_sent: BTreeMap<u8, Duration>,
}

impl MonitorStats {
    /// Empty statistics for a line at `baud_rate`, whose timestamps are
    /// uncertain by `resolution`.
    #[must_use]
    pub fn new(baud_rate: u32, resolution: Duration) -> Self {
        Self {
            timing: TimingQuality {
                resolution,
                octet_time: octet_time(baud_rate),
                ..TimingQuality::default()
            },
            ..Self::default()
        }
    }

    /// The station's statistics, if it has been seen.
    #[must_use]
    pub fn station(&self, address: u8) -> Option<&StationStats> {
        self.stations.get(&address)
    }

    /// Addresses of the stations that sent at least one valid frame.
    pub fn present_stations(&self) -> impl Iterator<Item = u8> + '_ {
        self.stations
            .iter()
            .filter(|(_, station)| station.is_present())
            .map(|(address, _)| *address)
    }

    /// Count one read of `octets` octets from the port.
    pub fn record_read(&mut self, octets: usize) {
        let octets = u64::try_from(octets).unwrap_or(u64::MAX);
        self.octets = self.octets.saturating_add(octets);
        self.timing.reads = self.timing.reads.saturating_add(1);
        self.timing.max_octets_per_read = self.timing.max_octets_per_read.max(octets);
    }

    /// Take in one event whose last octet arrived at `at`.
    ///
    /// Events must come in the order they happened. A frame's start is
    /// worked out from its wire length at the configured baud rate.
    pub fn record(&mut self, at: Duration, event: &LineEvent) {
        let start = match event {
            LineEvent::Frame { wire_length, .. }
            | LineEvent::Error {
                octets: wire_length,
                ..
            } => at.saturating_sub(self.wire_time(*wire_length)),
            LineEvent::Noise { octets } => at.saturating_sub(self.wire_time(*octets)),
            LineEvent::FrameAbort => at,
        };
        let regenerated_by = match event {
            LineEvent::Frame { header, .. } => Some(header.source),
            _ => None,
        };
        if !matches!(event, LineEvent::FrameAbort) {
            self.note_activity(start, at, regenerated_by);
        }
        match *event {
            LineEvent::Frame { header, .. } => self.record_frame(start, at, header),
            LineEvent::Error { kind, .. } => {
                let slot = match kind {
                    LineErrorKind::HeaderCrc => &mut self.errors.header_crc,
                    LineErrorKind::DataCrc => &mut self.errors.data_crc,
                    LineErrorKind::Framing => &mut self.errors.framing,
                };
                *slot = slot.saturating_add(1);
                // Whatever came before is no longer the frame straight
                // before the next one.
                self.last_frame = None;
            }
            LineEvent::Noise { octets } => {
                let octets = u64::try_from(octets).unwrap_or(u64::MAX);
                self.errors.noise_octets = self.errors.noise_octets.saturating_add(octets);
            }
            LineEvent::FrameAbort => {
                self.errors.frame_aborts = self.errors.frame_aborts.saturating_add(1);
                self.last_frame = None;
            }
        }
    }

    fn wire_time(&self, octets: usize) -> Duration {
        self.timing
            .octet_time
            .saturating_mul(u32::try_from(octets).unwrap_or(u32::MAX))
    }

    /// The silence from the last activity to `start`, and a lost token if
    /// it was long enough.
    fn note_activity(&mut self, start: Duration, end: Duration, source: Option<u8>) {
        if let Some(previous) = self.last_activity {
            let silence = start.saturating_sub(previous);
            self.silences.add(silence);
            if silence >= NO_TOKEN_SILENCE {
                self.lost_tokens = self.lost_tokens.saturating_add(1);
                self.last_lost_token = Some(LostToken {
                    at: start,
                    silence,
                    regenerated_by: source,
                });
                self.last_frame = None;
            }
        }
        self.last_activity = Some(self.last_activity.map_or(end, |last| last.max(end)));
    }

    fn record_frame(&mut self, start: Duration, end: Duration, header: FrameHeader) {
        let FrameHeader {
            frame_type,
            destination,
            source,
            ..
        } = header;
        self.frames.add(frame_type);
        if frame_type.is_cobs() {
            self.cobs_frames = self.cobs_frames.saturating_add(1);
        }
        let previous = self.last_frame.replace((end, header));

        let sender = self.stations.entry(source).or_default();
        sender.frames_sent.add(frame_type);
        sender.last_seen = Some(end);
        // The frame straight after a Token to this station: how soon it
        // took the token up.
        if let Some((token_end, token)) = previous {
            if token.frame_type == FrameType::TOKEN && token.destination == source {
                sender
                    .token_usage_delay
                    .add(start.saturating_sub(token_end));
            }
            if frame_type == FrameType::TOKEN
                && token.frame_type == FrameType::TOKEN
                && token.source == source
                && token.destination == destination
            {
                sender.token_retries = sender.token_retries.saturating_add(1);
            }
        }
        match frame_type {
            FrameType::TOKEN if destination != BROADCAST_MAC => {
                sender.tokens_passed = sender.tokens_passed.saturating_add(1);
                sender.next_station = Some(destination);
                if let Some(earlier) = self.last_token_sent.insert(source, start) {
                    sender.token_rotation.add(start.saturating_sub(earlier));
                }
                self.token_passes = self.token_passes.saturating_add(1);
                let receiver = self.stations.entry(destination).or_default();
                receiver.tokens_received = receiver.tokens_received.saturating_add(1);
            }
            FrameType::POLL_FOR_MASTER if destination != BROADCAST_MAC => {
                sender.max_master_inferred = Some(
                    sender
                        .max_master_inferred
                        .map_or(destination, |highest| highest.max(destination)),
                );
            }
            _ => {}
        }
        if destination != BROADCAST_MAC {
            let receiver = self.stations.entry(destination).or_default();
            receiver.frames_received.add(frame_type);
        }
    }
}
