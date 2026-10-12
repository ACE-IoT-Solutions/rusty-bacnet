//! `StreamDecoder` on structure-aware input.
//!
//! # Input
//!
//! A stream of chunks: valid frames of every type the codec handles, with 0
//! to 501 octets of data, written by `mstp_codec_fuzz::encode`; raw noise,
//! any octets; and inert noise, which has no 0x55 and so can't start a
//! candidate. A decoder with 1 to 575 octets of storage (the longest frame
//! is 511) takes it in pushes of arbitrary size, single octets through
//! `push`, with empty pushes, drains (empty pushes until `NeedMore`) and
//! resets (a drain, then `reset`) between them. A refused push is pushed
//! again in pieces that fit. What the steps leave is pushed the same way,
//! then drained.
//!
//! # Oracle
//!
//! The target keeps a shadow of the decoder's buffer, the stream octets from
//! the first one the decoder still holds to the last one pushed, and checks
//! each push against it:
//!
//! - Refusal: a push is refused exactly when it is longer than the free
//!   room, counting the frame the push commits, with those two sizes, and
//!   nothing is buffered.
//! - Never wedged: before every push there is room for one more octet,
//!   `NeedMore` leaves the storage short of full, and a drain ends within
//!   2 × capacity + 4 empty pushes.
//! - `NeedMore` only while the buffer is empty or the front of a candidate
//!   that still lacks octets and will fit: its header, once complete, passes
//!   its CRC and the standard-frame checks.
//! - `Invalid` throws away no possible preamble: each 0x55 it discards has a
//!   buffered octet after it that isn't 0xFF.
//! - `Malformed` drops a candidate at the front, with the error, discard and
//!   destination the decoder's docs give for it.
//! - `Frame` is the whole valid frame at the front, by
//!   `mstp_codec_fuzz::valid_frame`, with the same fields.
//! - The buffered length matches the shadow after every push, and so does
//!   the phase after `NeedMore` or a frame.
//!
//! At the end, the delivered frames are in stream order and don't overlap,
//! so none came out twice, and every guaranteed generated frame came out at
//! its offset. A generated frame is guaranteed when it fits the storage, no
//! reset fell inside it, and no 0x55 lies between the anchor and its first
//! octet. The anchor is the latest of the stream's start, the last reset at
//! or before the frame, and the end of the last frame delivered before it;
//! a delivered frame that runs past the frame's first octet hid it. From the
//! anchor the decoder starts fresh, and octets other than 0x55 can't start
//! a candidate, so nothing can hide the frame.
//!
//! Noise with a 0x55 can hide what follows it: it may hold a header that
//! passes its CRC and declares a length running into the next frame, and
//! the decoder then waits for, delivers or discards that candidate whole.
//! So can a frame too long for the storage, because the decoder drops only
//! its first octet and rescans its body as fresh octets (#1609); its
//! preamble is a 0x55 after the anchor, so the frames after it aren't
//! guaranteed. Once #1609 skips a dropped frame's body by count, the end of
//! such a frame can serve as an anchor too, and the `Invalid` and
//! `Malformed` discard checks change with it.

#![no_main]

use arbitrary::Arbitrary;
use bacnet_mstp_codec::{DecodeEvent, FrameError, StreamDecoder, StreamPhase};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, Header, HEADER_END, MAX_DATA, MAX_WIRE};

/// A stream stops taking chunks past this many octets, to keep runs quick.
const MAX_STREAM: usize = 8 * 1024;
/// Storage sizes run from 1 octet to this many.
const MAX_CAPACITY: usize = MAX_WIRE + 64;

#[derive(Debug, Arbitrary)]
struct Input {
    /// The storage size, 1 to `MAX_CAPACITY` (modulo).
    capacity: u16,
    chunks: Vec<Chunk>,
    steps: Vec<Step>,
}

#[derive(Debug, Arbitrary)]
enum Chunk {
    /// A valid frame. A COBS type folds into 0-31 and source 255 becomes
    /// 254; a type defined without data gets none, and a proprietary type at
    /// least the two octets of its vendor identifier. `length` is taken
    /// modulo 502, and `pattern` repeated to fill the data.
    Frame {
        frame_type: u8,
        destination: u8,
        source: u8,
        length: u16,
        pattern: Vec<u8>,
    },
    /// Any octets.
    Noise(Vec<u8>),
    /// Octets with each 0x55 turned into 0x54.
    Inert(Vec<u8>),
}

#[derive(Debug, Arbitrary)]
enum Step {
    /// Push the next 1 to `MAX_CAPACITY` octets (modulo) as one slice.
    Push(u16),
    /// One empty push.
    Empty,
    /// Empty pushes until `NeedMore`.
    Drain,
    /// A drain, then `reset`.
    Reset,
}

/// The generated octets, and the offset and length of each frame in them.
struct Stream {
    octets: Vec<u8>,
    frames: Vec<(usize, usize)>,
}

impl Stream {
    fn build(chunks: &[Chunk]) -> Self {
        let mut stream = Self {
            octets: Vec::new(),
            frames: Vec::new(),
        };
        for chunk in chunks {
            if stream.octets.len() >= MAX_STREAM {
                break;
            }
            match chunk {
                Chunk::Frame {
                    frame_type,
                    destination,
                    source,
                    length,
                    pattern,
                } => {
                    let raw = match *frame_type {
                        raw if wire::is_cobs(raw) => raw & 0x1F,
                        raw => raw,
                    };
                    let length = match usize::from(*length) % (MAX_DATA + 1) {
                        _ if wire::never_carries_data(raw) => 0,
                        length if wire::is_proprietary(raw) => length.max(2),
                        length => length,
                    };
                    let data = wire::fill(length, pattern);
                    let frame = wire::encode(raw, *destination, (*source).min(0xFE), &data);
                    stream.frames.push((stream.octets.len(), frame.len()));
                    stream.octets.extend_from_slice(&frame);
                }
                Chunk::Noise(octets) => stream.octets.extend_from_slice(octets),
                Chunk::Inert(octets) => stream.octets.extend(octets.iter().map(|&octet| {
                    if octet == 0x55 {
                        0x54
                    } else {
                        octet
                    }
                })),
            }
        }
        stream
    }
}

/// A `DecodeEvent`, copied out of the decoder's storage.
#[derive(Debug, PartialEq)]
enum Seen {
    NeedMore,
    Frame {
        consumed: usize,
        fields: [u8; 3],
        data: Vec<u8>,
    },
    Invalid {
        discard: usize,
    },
    Malformed {
        discard: usize,
        error: FrameError,
        destination: Option<u8>,
    },
    Error(FrameError),
}

impl From<DecodeEvent<'_>> for Seen {
    fn from(event: DecodeEvent<'_>) -> Self {
        match event {
            DecodeEvent::NeedMore => Self::NeedMore,
            DecodeEvent::Frame { frame, consumed } => Self::Frame {
                consumed,
                fields: [
                    frame.header.frame_type.to_raw(),
                    frame.header.destination,
                    frame.header.source,
                ],
                data: frame.data.to_vec(),
            },
            DecodeEvent::Invalid { discard } => Self::Invalid { discard },
            DecodeEvent::Malformed {
                discard,
                error,
                destination,
            } => Self::Malformed {
                discard,
                error,
                destination,
            },
            DecodeEvent::Error(error) => Self::Error(error),
        }
    }
}

/// A decoder fed from a stream, with the shadow of its buffer.
struct Run<'a> {
    decoder: StreamDecoder<'a>,
    capacity: usize,
    stream: &'a [u8],
    /// Stream octets pushed and accepted so far.
    pushed: usize,
    /// Stream offset of the first octet the decoder holds.
    start: usize,
    /// Wire length of the frame the last push delivered, which the next
    /// push commits.
    pending: usize,
    /// Offset and length of each delivered frame.
    delivered: Vec<(usize, usize)>,
    /// Stream offsets at which the decoder was reset.
    resets: Vec<usize>,
}

impl Run<'_> {
    /// Room for the next push, which first commits a delivered frame.
    fn free(&self) -> usize {
        self.capacity - (self.pushed - self.start - self.pending)
    }

    /// Push the next `length` stream octets, check what the decoder
    /// reports, and return it.
    fn push(&mut self, length: usize) -> Seen {
        let free = self.free();
        assert!(free >= 1, "full storage and nothing to report: wedged");
        let stream = self.stream;
        let input = &stream[self.pushed..self.pushed + length];
        let seen = Seen::from(match input {
            [octet] => self.decoder.push(*octet),
            _ => self.decoder.push_slice(input),
        });
        self.start += self.pending;
        self.pending = 0;
        if length > free {
            let refused = FrameError::BufferExhausted {
                needed: length,
                available: free,
            };
            assert_eq!(seen, Seen::Error(refused));
        } else {
            self.pushed += length;
            let shadow = &stream[self.start..self.pushed];
            match &seen {
                Seen::NeedMore => need_more(shadow, self.capacity),
                Seen::Invalid { discard } => {
                    invalid(shadow, *discard);
                    self.start += discard;
                }
                Seen::Malformed {
                    discard,
                    error,
                    destination,
                } => {
                    malformed(shadow, *discard, *error, *destination, self.capacity);
                    self.start += discard;
                }
                Seen::Frame {
                    consumed,
                    fields,
                    data,
                } => {
                    frame(shadow, *consumed, *fields, data);
                    self.delivered.push((self.start, *consumed));
                    self.pending = *consumed;
                }
                Seen::Error(error) => panic!("push of {length} with {free} free: {error:?}"),
            }
        }
        assert_eq!(self.decoder.buffered_len(), self.pushed - self.start);
        self.check_phase(&seen);
        seen
    }

    /// After `NeedMore` or a frame, the phase the shadow is in.
    fn check_phase(&self, seen: &Seen) {
        let expected = match seen {
            Seen::Frame { .. } => StreamPhase::Sync,
            Seen::NeedMore => match self.pushed - self.start {
                0 => StreamPhase::Sync,
                1 => StreamPhase::Preamble,
                buffered if buffered < HEADER_END => StreamPhase::Header,
                _ => StreamPhase::Data,
            },
            _ => return,
        };
        assert_eq!(self.decoder.phase(), expected);
    }

    /// Push the next `length` octets; if that is refused, push them again
    /// in pieces that fit.
    fn feed(&mut self, length: usize) {
        let end = self.pushed + length;
        if !matches!(self.push(length), Seen::Error(_)) {
            return;
        }
        while self.pushed < end {
            let piece = (end - self.pushed).min(self.free());
            self.push(piece);
        }
    }

    fn drain(&mut self) {
        for _ in 0..2 * self.capacity + 4 {
            if self.push(0) == Seen::NeedMore {
                return;
            }
        }
        panic!("a drain did not end");
    }

    fn reset(&mut self) {
        self.drain();
        self.decoder.reset();
        self.start = self.pushed;
        self.resets.push(self.pushed);
        assert_eq!(self.decoder.buffered_len(), 0);
        assert_eq!(self.decoder.phase(), StreamPhase::Sync);
    }
}

/// `NeedMore`: the buffer is empty, or the front of a candidate that still
/// lacks octets and will fit.
fn need_more(shadow: &[u8], capacity: usize) {
    assert!(
        shadow.len() < capacity,
        "NeedMore with full storage: wedged"
    );
    if shadow.is_empty() {
        return;
    }
    assert!(
        shadow.starts_with(&wire::PREAMBLE[..shadow.len().min(2)]),
        "NeedMore holding noise: {shadow:02X?}"
    );
    assert!(
        capacity >= HEADER_END,
        "NeedMore for a header that can't fit"
    );
    let header = Header::read(shadow);
    match header {
        Header::Incomplete => {}
        Header::Good { length, .. } if header.is_standard() => {
            let total = wire::wire_length(length);
            assert!(
                total <= capacity,
                "NeedMore for {total} octets in {capacity}"
            );
            assert!(shadow.len() < total, "NeedMore holding a whole candidate");
        }
        _ => panic!("NeedMore after a header to drop: {header:?}"),
    }
}

/// `Invalid`: noise. Each discarded 0x55 has a buffered octet after it that
/// isn't 0xFF, so none was the start of a preamble.
fn invalid(shadow: &[u8], discard: usize) {
    assert!(
        (1..=shadow.len()).contains(&discard),
        "Invalid discards {discard}"
    );
    for (index, &octet) in shadow[..discard].iter().enumerate() {
        if octet == 0x55 {
            assert!(
                shadow.get(index + 1).is_some_and(|&next| next != 0xFF),
                "Invalid discarded a possible preamble at {index}: {shadow:02X?}"
            );
        }
    }
}

/// `Malformed`: the candidate at the front was dropped. Its destination is
/// known once the header CRC has passed. A bad data CRC drops the whole
/// frame, anything else one octet.
fn malformed(
    shadow: &[u8],
    discard: usize,
    error: FrameError,
    destination: Option<u8>,
    capacity: usize,
) {
    assert!(!shadow.is_empty(), "Malformed with nothing buffered");
    assert!(
        shadow.starts_with(&wire::PREAMBLE[..shadow.len().min(2)]),
        "Malformed for noise: {shadow:02X?}"
    );
    let header = Header::read(shadow);
    let known = match header {
        Header::Good { destination, .. } => Some(destination),
        _ => None,
    };
    assert_eq!(destination, known, "destination for {error:?}");
    let cobs = matches!(header, Header::Good { frame_type, .. } if wire::is_cobs(frame_type));
    let expected = match (error, header) {
        (FrameError::HeaderCrcMismatch, Header::BadCrc) => 1,
        (FrameError::UnsupportedCobsFrame { raw_type }, Header::Good { frame_type, .. })
            if cobs && raw_type == frame_type =>
        {
            1
        }
        (FrameError::BroadcastSource, Header::Good { source: 0xFF, .. }) if !cobs => 1,
        (
            FrameError::Oversize { length, maximum },
            Header::Good {
                source,
                length: declared,
                ..
            },
        ) if !cobs && source != 0xFF && length == declared && length > MAX_DATA => {
            assert_eq!(maximum, MAX_DATA);
            1
        }
        (
            FrameError::FrameTooLong {
                needed,
                capacity: reported,
            },
            _,
        ) if reported == capacity && needed > capacity => {
            match header {
                Header::Incomplete => assert_eq!(needed, HEADER_END),
                Header::Good { length, .. } if header.is_standard() => {
                    assert_eq!(needed, wire::wire_length(length))
                }
                _ => panic!("FrameTooLong for header {header:?}"),
            }
            1
        }
        (FrameError::DataCrcMismatch { data_length }, Header::Good { length, .. })
            if header.is_standard() && data_length == length =>
        {
            let total = wire::wire_length(length);
            assert!(
                total <= shadow.len(),
                "DataCrcMismatch before the frame is whole"
            );
            assert_eq!(
                wire::valid_frame(shadow),
                None,
                "DataCrcMismatch for a good frame"
            );
            total
        }
        _ => panic!("Malformed {error:?} for {header:?}: {shadow:02X?}"),
    };
    assert_eq!(discard, expected, "discard for {error:?}");
}

/// `Frame`: the whole valid frame at the front, with its fields.
fn frame(shadow: &[u8], consumed: usize, fields: [u8; 3], data: &[u8]) {
    assert_eq!(
        wire::valid_frame(shadow),
        Some(consumed),
        "delivered something other than the frame at the front: {shadow:02X?}"
    );
    assert_eq!(fields, [shadow[2], shadow[3], shadow[4]]);
    assert_eq!(wire::wire_length(data.len()), consumed);
    assert_eq!(data, &shadow[HEADER_END..HEADER_END + data.len()]);
}

/// Frames come out in order, once each, and every guaranteed generated
/// frame came out at its offset (see the module docs).
fn check_delivered(
    stream: &Stream,
    capacity: usize,
    delivered: &[(usize, usize)],
    resets: &[usize],
) {
    for pair in delivered.windows(2) {
        assert!(pair[0].0 + pair[0].1 <= pair[1].0, "overlap: {pair:?}");
    }
    let (mut next_delivered, mut next_reset, mut anchor) = (0, 0, 0);
    for &(offset, length) in &stream.frames {
        while let Some(&(start, size)) = delivered.get(next_delivered) {
            if start >= offset {
                break;
            }
            anchor = anchor.max(start + size);
            next_delivered += 1;
        }
        while let Some(&reset) = resets.get(next_reset) {
            if reset > offset {
                break;
            }
            anchor = anchor.max(reset);
            next_reset += 1;
        }
        let hidden = anchor > offset;
        let straddled = resets
            .get(next_reset)
            .is_some_and(|&reset| reset < offset + length);
        if length <= capacity
            && !hidden
            && !straddled
            && !stream.octets[anchor..offset].contains(&0x55)
        {
            assert_eq!(
                delivered.get(next_delivered),
                Some(&(offset, length)),
                "the frame at {offset} ({length} octets) didn't come out"
            );
        }
    }
}

fuzz_target!(
    init: wire::self_test(),
    |input: Input| {
        let stream = Stream::build(&input.chunks);
        let capacity = 1 + usize::from(input.capacity) % MAX_CAPACITY;
        let mut storage = vec![0u8; capacity];
        let mut run = Run {
            decoder: StreamDecoder::new(&mut storage),
            capacity,
            stream: &stream.octets,
            pushed: 0,
            start: 0,
            pending: 0,
            delivered: Vec::new(),
            resets: Vec::new(),
        };
        assert_eq!(run.decoder.capacity(), capacity);
        for step in &input.steps {
            match *step {
                Step::Push(length) => {
                    let left = stream.octets.len() - run.pushed;
                    run.feed((1 + usize::from(length) % MAX_CAPACITY).min(left));
                }
                Step::Empty => {
                    run.push(0);
                }
                Step::Drain => run.drain(),
                Step::Reset => run.reset(),
            }
        }
        run.feed(stream.octets.len() - run.pushed);
        run.drain();
        check_delivered(&stream, capacity, &run.delivered, &run.resets);
    }
);
