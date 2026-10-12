//! What the MS/TP codec's fuzz targets share.
//!
//! [`encode`] and [`valid_frame`] are written from the frame layout (Clause
//! 9.3) and the two CRC definitions (Clause 9.6), with bit-at-a-time CRCs
//! rather than the codec's tables, and without calling the codec. The targets
//! check the codec against them, so a fault that the codec's encoder and
//! decoder share can't cancel out in a round trip. [`self_test`] pins them to
//! two frames written out by hand.
//!
//! [`check_sinks`] encodes one frame through every `FrameSink` the codec
//! implements and checks what each one holds afterwards.

use std::fmt::Debug;

use bacnet_mstp_codec::FrameSink;

/// The two preamble octets.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];
/// Octets up to the data field: the preamble, then type, destination,
/// source, two length octets and the header CRC.
pub const HEADER_END: usize = 8;
/// Largest data field of a standard frame.
pub const MAX_DATA: usize = 501;
/// Longest standard frame on the wire.
pub const MAX_WIRE: usize = HEADER_END + MAX_DATA + 2;
/// The octet every fixed sink is filled with before an encode, so that a
/// write outside the frame shows.
pub const SENTINEL: u8 = 0xA5;
/// Capacity of the heapless sink the targets use: a little more than the
/// longest frame, so its prefill decides whether a frame fits.
pub const HEAPLESS_CAPACITY: usize = MAX_WIRE + 9;

/// The header CRC: CRC-8 with polynomial x^8 + x^7 + 1, shifted out least
/// significant bit first from an all-ones register, and sent complemented.
pub fn header_crc(fields: &[u8]) -> u8 {
    let mut register: u8 = 0xFF;
    for &octet in fields {
        for bit in 0..8 {
            let feedback = (register ^ (octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                // x^7 + 1, bit-reversed, is again 0x81.
                register ^= 0x81;
            }
        }
    }
    !register
}

/// The data CRC: CRC-16 with polynomial x^16 + x^12 + x^5 + 1, shifted out
/// least significant bit first from an all-ones register, and sent
/// complemented, low octet first.
pub fn data_crc(data: &[u8]) -> u16 {
    let mut register: u16 = 0xFFFF;
    for &octet in data {
        for bit in 0..8 {
            let feedback = (register ^ u16::from(octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                // x^12 + x^5 + 1, bit-reversed.
                register ^= 0x8408;
            }
        }
    }
    !register
}

/// Wire length of a standard frame with `data_length` octets of data: no
/// data CRC follows an empty data field.
pub const fn wire_length(data_length: usize) -> usize {
    if data_length == 0 {
        HEADER_END
    } else {
        HEADER_END + data_length + 2
    }
}

/// Whether `raw` is a COBS-encoded frame type (32-127), which the codec
/// refuses in both directions.
pub const fn is_cobs(raw: u8) -> bool {
    matches!(raw, 0x20..=0x7F)
}

/// Whether frames of type `raw` are defined without data: types 0 to 2 (the
/// token and the two poll-for-master frames) and 7 (the postponed reply).
pub const fn never_carries_data(raw: u8) -> bool {
    matches!(raw, 0 | 1 | 2 | 7)
}

/// Whether `raw` is a proprietary frame type (128-255), whose data opens
/// with a 2-octet vendor identifier.
pub const fn is_proprietary(raw: u8) -> bool {
    raw >= 0x80
}

/// The wire octets of a frame, built field by field with no checks. The
/// data must be at most 65,535 octets.
pub fn encode(raw_type: u8, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let [high, low] = u16::try_from(data.len())
        .expect("data length fits the length field")
        .to_be_bytes();
    let fields = [raw_type, destination, source, high, low];
    let mut wire = Vec::with_capacity(wire_length(data.len()));
    wire.extend_from_slice(&PREAMBLE);
    wire.extend_from_slice(&fields);
    wire.push(header_crc(&fields));
    if !data.is_empty() {
        wire.extend_from_slice(data);
        wire.extend_from_slice(&data_crc(data).to_le_bytes());
    }
    wire
}

/// A frame header read from the octets after a preamble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// Fewer than the eight octets of preamble and header.
    Incomplete,
    /// The header CRC doesn't match.
    BadCrc,
    /// The header CRC matches.
    Good {
        /// Frame type octet.
        frame_type: u8,
        /// Destination address.
        destination: u8,
        /// Source address.
        source: u8,
        /// The data length the header declares.
        length: usize,
    },
}

impl Header {
    /// Read the header of the candidate at the start of `octets`, which the
    /// caller has checked starts with the preamble.
    pub fn read(octets: &[u8]) -> Self {
        let Some(header) = octets.get(..HEADER_END) else {
            return Self::Incomplete;
        };
        if header_crc(&header[2..7]) != header[7] {
            return Self::BadCrc;
        }
        Self::Good {
            frame_type: header[2],
            destination: header[3],
            source: header[4],
            length: usize::from(u16::from_be_bytes([header[5], header[6]])),
        }
    }

    /// Whether a frame with this header can be a valid standard frame: not
    /// a COBS type, a source other than 255 and at most 501 octets of data.
    /// `false` for a header that is incomplete or fails its CRC.
    pub fn is_standard(self) -> bool {
        matches!(self, Self::Good { frame_type, source, length, .. }
            if !is_cobs(frame_type) && source != 0xFF && length <= MAX_DATA)
    }
}

/// The wire length of the valid standard frame at the start of `octets`,
/// or `None` when there isn't one there whole. Like the codec's decoder it
/// accepts data on a type defined without any and a proprietary frame too
/// short for its vendor identifier.
pub fn valid_frame(octets: &[u8]) -> Option<usize> {
    if !octets.starts_with(&PREAMBLE) {
        return None;
    }
    let header = Header::read(octets);
    let Header::Good { length, .. } = header else {
        return None;
    };
    if !header.is_standard() {
        return None;
    }
    let total = wire_length(length);
    let frame = octets.get(..total)?;
    if length != 0 {
        let data = &frame[HEADER_END..HEADER_END + length];
        if data_crc(data).to_le_bytes() != frame[HEADER_END + length..] {
            return None;
        }
    }
    Some(total)
}

/// `length` octets of `pattern` repeated, or a counting pattern when it is
/// empty. Lets a short input ask for a long data field.
pub fn fill(length: usize, pattern: &[u8]) -> Vec<u8> {
    if pattern.is_empty() {
        return (0..length)
            .map(|index| (index as u8).wrapping_mul(31))
            .collect();
    }
    pattern.iter().copied().cycle().take(length).collect()
}

/// Check [`encode`] and [`valid_frame`] against two frames from the codec's
/// documentation, a Token and a Data Not Expecting Reply from slave 200, and
/// [`data_crc`] against the worked example of Annex G.2: data 01 22 30 goes
/// out with the CRC octets 10 BD.
pub fn self_test() {
    let token = [0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C];
    let data = [
        0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF, 0x01, 0x00, 0x9F, 0x16,
    ];
    assert_eq!(encode(0x00, 0x10, 0x05, &[]), token);
    assert_eq!(encode(0x06, 0x01, 0xC8, &[0x01, 0x00]), data);
    assert_eq!(valid_frame(&token), Some(token.len()));
    assert_eq!(valid_frame(&data), Some(data.len()));
    assert_eq!(valid_frame(&data[..data.len() - 1]), None);
    assert_eq!(data_crc(&[0x01, 0x22, 0x30]).to_le_bytes(), [0x10, 0xBD]);
}

/// How one encode exercises the sinks: the slice's room relative to the
/// frame, what the growable sinks already hold, and how full the heapless
/// sink is.
#[derive(Debug, arbitrary::Arbitrary)]
pub struct SinkKnobs {
    /// The slice sinks get the frame's wire length plus this, at least 0.
    pub slack: i8,
    /// The `Vec` and `BytesMut` sinks start with these octets.
    pub prefix: Vec<u8>,
    /// Octets the heapless sink already holds, modulo its capacity plus one.
    pub heapless_used: u16,
}

/// Encode one frame through every sink and check each: what the call
/// returns, the octets it wrote, and that a refusal left the sink as it was.
///
/// `expected` is the frame's wire octets, or the error its checks give,
/// and `wire_length` its length even when refused. `exhausted` builds the
/// error for a fixed sink without room. Returns the octets each successful
/// encode produced, for the caller to decode.
pub fn check_sinks<E>(
    expected: &Result<Vec<u8>, E>,
    wire_length: usize,
    knobs: &SinkKnobs,
    exhausted: impl Fn(usize, usize) -> E,
    encode_into: impl Fn(&mut [u8]) -> Result<usize, E>,
    encode_to_sink: impl Fn(&mut dyn FrameSink) -> Result<usize, E>,
) -> Vec<Vec<u8>>
where
    E: Clone + PartialEq + Debug,
{
    let mut outputs = Vec::new();
    let room = wire_length.saturating_add_signed(isize::from(knobs.slack));
    // A fixed sink with `room` octets free.
    let fixed = |room: usize| match expected {
        Ok(wire) if wire.len() > room => Err(exhausted(wire.len(), room)),
        Ok(wire) => Ok(wire.len()),
        Err(error) => Err(error.clone()),
    };

    // A slice through encode_into: the frame at the front, the rest untouched.
    let mut slice = vec![SENTINEL; room];
    let result = encode_into(&mut slice);
    assert_eq!(result, fixed(room), "slice of {room} octets");
    check_fixed(&slice, result.is_ok(), expected, &mut outputs);

    // The same through the sink trait, which advances the slice.
    let mut slice = vec![SENTINEL; room];
    let mut tail: &mut [u8] = &mut slice;
    let result = encode_to_sink(&mut tail);
    let left = tail.len();
    assert_eq!(result, fixed(room), "slice sink of {room} octets");
    assert_eq!(left, room - result.as_ref().map_or(0, |length| *length));
    check_fixed(&slice, result.is_ok(), expected, &mut outputs);

    // The growable sinks keep what they held and append the frame; a refused
    // frame fails its checks before anything is written.
    let mut growable = |held: &[u8], result: Result<usize, E>| {
        assert_eq!(
            result,
            expected.as_ref().map(Vec::len).map_err(Clone::clone)
        );
        let (prefix, written) = held.split_at(knobs.prefix.len());
        assert_eq!(prefix, knobs.prefix);
        match expected {
            Ok(wire) => {
                assert_eq!(written, &wire[..]);
                outputs.push(written.to_vec());
            }
            Err(_) => assert!(written.is_empty(), "a refused frame left octets behind"),
        }
    };
    let mut vec = knobs.prefix.clone();
    let result = encode_to_sink(&mut vec);
    growable(&vec, result);
    let mut bytes = bytes::BytesMut::from(&knobs.prefix[..]);
    let result = encode_to_sink(&mut bytes);
    growable(&bytes, result);

    // A heapless vector with some of its capacity used.
    let used = usize::from(knobs.heapless_used) % (HEAPLESS_CAPACITY + 1);
    let mut heapless = heapless::Vec::<u8, HEAPLESS_CAPACITY>::new();
    heapless
        .resize(used, SENTINEL)
        .expect("prefill fits the capacity");
    let result = encode_to_sink(&mut heapless);
    assert_eq!(
        result,
        fixed(HEAPLESS_CAPACITY - used),
        "heapless, {used} used"
    );
    assert!(heapless[..used].iter().all(|&octet| octet == SENTINEL));
    match (&result, expected) {
        (Ok(_), Ok(wire)) => {
            assert_eq!(heapless[used..], wire[..]);
            outputs.push(heapless[used..].to_vec());
        }
        _ => assert_eq!(heapless.len(), used, "a refused frame left octets behind"),
    }
    outputs
}

/// After an encode into a sentinel-filled slice: on success the frame is at
/// the front and the rest is untouched, and on refusal nothing changed.
fn check_fixed<E>(
    slice: &[u8],
    accepted: bool,
    expected: &Result<Vec<u8>, E>,
    outputs: &mut Vec<Vec<u8>>,
) {
    let written = match (accepted, expected) {
        (true, Ok(wire)) => {
            assert_eq!(slice[..wire.len()], wire[..]);
            outputs.push(wire.clone());
            wire.len()
        }
        _ => 0,
    };
    assert!(
        slice[written..].iter().all(|&octet| octet == SENTINEL),
        "an encode wrote outside its frame, or a refused one wrote anything"
    );
}
