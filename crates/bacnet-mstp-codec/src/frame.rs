//! Frame types, headers, and one-shot encode and decode.

use crate::{
    crc16, crc16_valid, crc8, crc8_valid, FrameError, FrameSink, BROADCAST_MAC, HEADER_LENGTH,
    MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

/// Octets before the data field: the preamble and the header with its CRC.
pub(crate) const DATA_OFFSET: usize = PREAMBLE.len() + HEADER_LENGTH;
/// The data CRC that follows a non-empty data field.
const DATA_CRC_LENGTH: usize = 2;

/// Wire length of a standard frame whose data field is `data_length` octets.
pub(crate) const fn wire_length(data_length: usize) -> usize {
    if data_length == 0 {
        DATA_OFFSET
    } else {
        DATA_OFFSET + data_length + DATA_CRC_LENGTH
    }
}

/// MS/TP frame type octet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Token frame.
    Token = 0x00,
    /// Poll For Master frame.
    PollForMaster = 0x01,
    /// Reply To Poll For Master frame.
    ReplyToPollForMaster = 0x02,
    /// Test Request frame.
    TestRequest = 0x03,
    /// Test Response frame.
    TestResponse = 0x04,
    /// BACnet data expecting a reply.
    BACnetDataExpectingReply = 0x05,
    /// BACnet data not expecting a reply.
    BACnetDataNotExpectingReply = 0x06,
    /// Reply postponed frame.
    ReplyPostponed = 0x07,
    /// Any other type octet: reserved, proprietary or COBS-encoded.
    ///
    /// [`FrameType::from_raw`] never returns it for 0-7. A value built by hand
    /// with 0-7 behaves as the named type in every check.
    Unknown(u8),
}

impl FrameType {
    /// Convert a raw frame type octet to its typed representation.
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Token,
            1 => Self::PollForMaster,
            2 => Self::ReplyToPollForMaster,
            3 => Self::TestRequest,
            4 => Self::TestResponse,
            5 => Self::BACnetDataExpectingReply,
            6 => Self::BACnetDataNotExpectingReply,
            7 => Self::ReplyPostponed,
            value => Self::Unknown(value),
        }
    }

    /// Return this frame type's wire value.
    pub const fn to_raw(self) -> u8 {
        match self {
            Self::Token => 0,
            Self::PollForMaster => 1,
            Self::ReplyToPollForMaster => 2,
            Self::TestRequest => 3,
            Self::TestResponse => 4,
            Self::BACnetDataExpectingReply => 5,
            Self::BACnetDataNotExpectingReply => 6,
            Self::ReplyPostponed => 7,
            Self::Unknown(value) => value,
        }
    }

    /// Whether a frame of this type may carry a data field.
    ///
    /// False only for the four types defined without data: [`Token`](Self::Token),
    /// [`PollForMaster`](Self::PollForMaster),
    /// [`ReplyToPollForMaster`](Self::ReplyToPollForMaster) and
    /// [`ReplyPostponed`](Self::ReplyPostponed) (Clauses 9.3.1-9.3.3, 9.3.8).
    /// The reserved and proprietary types this crate passes through may carry
    /// data.
    pub const fn allows_data(self) -> bool {
        !matches!(self.to_raw(), 0 | 1 | 2 | 7)
    }

    /// Whether this type is in the COBS-encoded range, 32-127 (Clause 9.3).
    pub const fn is_cobs(self) -> bool {
        let raw = self.to_raw();
        raw >= 0x20 && raw <= 0x7F
    }
}

/// The addressing fields of an MS/TP frame header.
///
/// The length field is not stored: it is always the length of the data that
/// goes with the header, [`Frame::data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Frame type.
    pub frame_type: FrameType,
    /// Destination station address; 255 is broadcast.
    pub destination: u8,
    /// Source station address.
    pub source: u8,
}

impl FrameHeader {
    /// Construct a header.
    pub const fn new(frame_type: FrameType, destination: u8, source: u8) -> Self {
        Self {
            frame_type,
            destination,
            source,
        }
    }
}

/// A frame whose data borrows from the caller or from decoder storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// The data field: an NPDU, test data or proprietary content. Its length
    /// is the frame's length field.
    pub data: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Construct a frame.
    pub const fn new(frame_type: FrameType, destination: u8, source: u8, data: &'a [u8]) -> Self {
        Self {
            header: FrameHeader::new(frame_type, destination, source),
            data,
        }
    }

    /// Encode this frame at the start of `output` and return its wire length.
    ///
    /// The frame is checked first, as described in the [crate docs](crate):
    /// no COBS type, at most [`MAX_STANDARD_MPDU_DATA`] octets of data, a
    /// source other than 255, and no data on a type that never carries any.
    /// Nothing is written unless the frame passes and fits.
    ///
    /// ```
    /// use bacnet_mstp_codec::{Frame, FrameError, FrameType};
    ///
    /// let mut wire = [0u8; 8];
    /// let length = Frame::new(FrameType::Token, 0x10, 0x05, &[]).encode_into(&mut wire)?;
    /// assert_eq!(wire[..length], [0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C]);
    ///
    /// // A Token never carries data.
    /// assert_eq!(
    ///     Frame::new(FrameType::Token, 0x10, 0x05, &[1]).encode_into(&mut wire),
    ///     Err(FrameError::UnexpectedData { raw_type: 0, length: 1 }),
    /// );
    /// # Ok::<(), FrameError>(())
    /// ```
    pub fn encode_into(self, output: &mut [u8]) -> Result<usize, FrameError> {
        let mut sink = output;
        self.encode_to_sink(&mut sink)
    }

    /// Encode this frame into `sink` and return its wire length.
    ///
    /// The checks are those of [`Frame::encode_into`]. When the sink reports a
    /// fixed capacity, a frame that does not fit fails before anything is
    /// written. A growable sink is written directly, so if one of its writes
    /// fails, the octets written before it stay in the sink; see
    /// [`FrameSink`].
    pub fn encode_to_sink<S: FrameSink + ?Sized>(self, sink: &mut S) -> Result<usize, FrameError> {
        let total = self.checked_wire_length()?;
        if let Some(available) = sink.remaining_capacity() {
            if available < total {
                return Err(FrameError::BufferExhausted {
                    needed: total,
                    available,
                });
            }
        }
        let length = self.data.len();
        let fields = [
            self.header.frame_type.to_raw(),
            self.header.destination,
            self.header.source,
            (length >> 8) as u8,
            length as u8,
        ];
        let mut head = [0u8; DATA_OFFSET];
        head[..PREAMBLE.len()].copy_from_slice(&PREAMBLE);
        head[PREAMBLE.len()..DATA_OFFSET - 1].copy_from_slice(&fields);
        head[DATA_OFFSET - 1] = crc8(&fields);
        sink.write(&head)?;
        if length != 0 {
            sink.write(self.data)?;
            sink.write(&crc16(self.data).to_le_bytes())?;
        }
        Ok(total)
    }

    /// Apply the encode checks and return the frame's wire length.
    fn checked_wire_length(&self) -> Result<usize, FrameError> {
        let frame_type = self.header.frame_type;
        let length = self.data.len();
        if frame_type.is_cobs() {
            return Err(FrameError::UnsupportedCobsFrame {
                raw_type: frame_type.to_raw(),
            });
        }
        if length > MAX_STANDARD_MPDU_DATA {
            return Err(FrameError::Oversize {
                length,
                maximum: MAX_STANDARD_MPDU_DATA,
            });
        }
        if self.header.source == BROADCAST_MAC {
            return Err(FrameError::BroadcastSource);
        }
        if length != 0 && !frame_type.allows_data() {
            return Err(FrameError::UnexpectedData {
                raw_type: frame_type.to_raw(),
                length,
            });
        }
        Ok(wire_length(length))
    }
}

/// Decode one frame from `input`, which must start with the preamble.
///
/// Returns the frame and the number of octets it occupies; anything after
/// them is ignored. The checks, in order: preamble, header CRC, COBS type,
/// source 255, length limit, completeness, data CRC. The [crate docs](crate)
/// explain what is deliberately not checked.
///
/// ```
/// use bacnet_mstp_codec::{decode_frame, FrameError, FrameType};
///
/// // Data Not Expecting Reply from slave 200 to station 1, data 01 00.
/// let wire = [0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF, 0x01, 0x00, 0x9F, 0x16];
/// let (frame, consumed) = decode_frame(&wire)?;
/// assert_eq!(frame.header.frame_type, FrameType::BACnetDataNotExpectingReply);
/// assert_eq!((frame.header.destination, frame.header.source), (1, 200));
/// assert_eq!(frame.data, [0x01, 0x00]);
/// assert_eq!(consumed, wire.len());
///
/// // Without its data CRC the frame is incomplete.
/// assert!(matches!(decode_frame(&wire[..10]), Err(FrameError::Truncated { .. })));
/// # Ok::<(), FrameError>(())
/// ```
pub fn decode_frame(input: &[u8]) -> Result<(Frame<'_>, usize), FrameError> {
    if input.len() < DATA_OFFSET {
        return Err(FrameError::TooShort {
            needed: DATA_OFFSET,
            available: input.len(),
        });
    }
    if input[..PREAMBLE.len()] != PREAMBLE {
        return Err(FrameError::InvalidPreamble {
            first: input[0],
            second: input[1],
        });
    }
    if !crc8_valid(&input[PREAMBLE.len()..DATA_OFFSET]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let frame_type = FrameType::from_raw(input[2]);
    if frame_type.is_cobs() {
        return Err(FrameError::UnsupportedCobsFrame { raw_type: input[2] });
    }
    let (destination, source) = (input[3], input[4]);
    if source == BROADCAST_MAC {
        return Err(FrameError::BroadcastSource);
    }
    let data_length = usize::from(u16::from_be_bytes([input[5], input[6]]));
    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::Oversize {
            length: data_length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    let total = wire_length(data_length);
    if input.len() < total {
        return Err(FrameError::Truncated {
            data_length,
            available: input.len() - DATA_OFFSET,
        });
    }
    if data_length != 0 && !crc16_valid(&input[DATA_OFFSET..total]) {
        return Err(FrameError::DataCrcMismatch { data_length });
    }
    let frame = Frame {
        header: FrameHeader::new(frame_type, destination, source),
        data: &input[DATA_OFFSET..DATA_OFFSET + data_length],
    };
    Ok((frame, total))
}

/// Return the offset of the first preamble in `data`, if it holds one.
pub fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|pair| pair == PREAMBLE)
}
