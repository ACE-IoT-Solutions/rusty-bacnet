//! Frame types, headers, and encoding.

use crate::cobs;
use crate::{
    crc16, crc8, FrameError, FrameSink, BROADCAST_MAC, HEADER_LENGTH, MAX_COBS_LENGTH,
    MAX_STANDARD_MPDU_DATA, MIN_COBS_LENGTH, PREAMBLE,
};

/// Octets before the data field: the preamble and the header with its CRC.
pub(crate) const DATA_OFFSET: usize = PREAMBLE.len() + HEADER_LENGTH;
/// What follows a non-empty data field: the two-octet data CRC of a standard
/// frame, or the two octets an extended frame's Length leaves out.
const TRAILER_LENGTH: usize = 2;

/// Wire length of a frame whose header declares `length`: the data and its
/// CRC for a standard frame, both encoded fields for an extended one, whose
/// Length counts them less two (Clause 9.3).
pub(crate) const fn wire_length(length: usize) -> usize {
    if length == 0 {
        DATA_OFFSET
    } else {
        DATA_OFFSET + length + TRAILER_LENGTH
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
    /// BACnet extended data expecting a reply: an NPDU too long for a
    /// standard frame, COBS-encoded (Clause 9.3.9).
    BACnetExtendedDataExpectingReply = 0x20,
    /// BACnet extended data not expecting a reply: an NPDU too long for a
    /// standard frame, COBS-encoded (Clause 9.3.10).
    BACnetExtendedDataNotExpectingReply = 0x21,
    /// Any other type octet: reserved (8-31), reserved and COBS-encoded
    /// (34-127), or proprietary (128-255).
    ///
    /// [`FrameType::from_raw`] never returns it for a named type. If one is
    /// built by hand, the codec's checks still treat it as the named type,
    /// because they look at the raw value; `==` does not, since equality is
    /// structural (`Unknown(0) != Token`).
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
            0x20 => Self::BACnetExtendedDataExpectingReply,
            0x21 => Self::BACnetExtendedDataNotExpectingReply,
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
            Self::BACnetExtendedDataExpectingReply => 0x20,
            Self::BACnetExtendedDataNotExpectingReply => 0x21,
            Self::Unknown(value) => value,
        }
    }

    /// Whether a frame of this type may carry a data field.
    ///
    /// False only for the four types defined without data: [`Token`](Self::Token),
    /// [`PollForMaster`](Self::PollForMaster),
    /// [`ReplyToPollForMaster`](Self::ReplyToPollForMaster) and
    /// [`ReplyPostponed`](Self::ReplyPostponed) (Clauses 9.3.1-9.3.3, 9.3.8).
    /// Every other type may, including the reserved types 8-31, which the
    /// codec passes through without policing their content.
    pub const fn allows_data(self) -> bool {
        !matches!(self.to_raw(), 0 | 1 | 2 | 7)
    }

    /// Whether this type is in the COBS-encoded range, 32-127 (Clause 9.3).
    /// Its Length must fall within
    /// [`MIN_COBS_LENGTH`]..=[`MAX_COBS_LENGTH`] (Clause 9.5.8).
    pub const fn is_cobs(self) -> bool {
        let raw = self.to_raw();
        raw >= 0x20 && raw <= 0x7F
    }

    /// Whether this is one of the two extended data types, 32 and 33: the
    /// COBS-encoded types the codec encodes and decodes, which carry BACnet
    /// NPDUs of 502 to [`MAX_EXTENDED_MPDU_DATA`](crate::MAX_EXTENDED_MPDU_DATA)
    /// octets (Clauses 9.3.9, 9.3.10). The rest of the COBS range, 34-127,
    /// is reserved.
    pub const fn is_extended_data(self) -> bool {
        matches!(self.to_raw(), 0x20 | 0x21)
    }

    /// Whether this type is in the vendor-proprietary range, 128-255. Such a
    /// frame's data opens with a 2-octet vendor identifier (Clause 9.3.11).
    pub const fn is_proprietary(self) -> bool {
        self.to_raw() >= 0x80
    }
}

/// The addressing fields of an MS/TP frame header.
///
/// The length field is not stored: it follows from the data that goes with
/// the header, [`Frame::data`], and the frame type.
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

/// Check the header fields that both directions check, in the order they
/// check them (Clause 9.5.8): the source, then the Length against the bounds
/// of the type's class, then whether the codec supports a COBS type. So
/// [`FrameError::UnsupportedCobsFrame`] always comes with a good header.
pub(crate) fn check_header_fields(
    frame_type: FrameType,
    source: u8,
    length: usize,
) -> Result<(), FrameError> {
    if source == BROADCAST_MAC {
        return Err(FrameError::BroadcastSource);
    }
    if frame_type.is_cobs() {
        if !(MIN_COBS_LENGTH..=MAX_COBS_LENGTH).contains(&length) {
            return Err(FrameError::InvalidCobsLength { length });
        }
        if !frame_type.is_extended_data() {
            return Err(FrameError::UnsupportedCobsFrame {
                raw_type: frame_type.to_raw(),
            });
        }
    } else if length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::Oversize {
            length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    Ok(())
}

/// A frame whose data borrows from the caller or from decoder storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// The data field as the sender's client gave it: an NPDU, test data or
    /// proprietary content. An extended frame carries it COBS-encoded on the
    /// wire; here it is decoded.
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
    /// a source other than 255; for a standard type, at most
    /// [`MAX_STANDARD_MPDU_DATA`] octets of data, none on a type that never
    /// carries any, and a vendor identifier's two octets on a proprietary
    /// type; for a COBS type, a Length within the COBS bounds and one of the
    /// two extended data types. Nothing is written unless the frame passes
    /// and fits.
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
        let length = self.checked_length()?;
        let total = wire_length(length);
        if let Some(available) = sink.remaining_capacity() {
            if available < total {
                return Err(FrameError::BufferExhausted {
                    needed: total,
                    available,
                });
            }
        }
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
        if self.header.frame_type.is_cobs() {
            cobs::write_fields(self.data, sink)?;
        } else if length != 0 {
            sink.write(self.data)?;
            sink.write(&crc16(self.data).to_le_bytes())?;
        }
        Ok(total)
    }

    /// Apply the encode checks and return the frame's Length field.
    fn checked_length(&self) -> Result<usize, FrameError> {
        let frame_type = self.header.frame_type;
        let length = if frame_type.is_cobs() {
            cobs::frame_length(self.data)
        } else {
            self.data.len()
        };
        check_header_fields(frame_type, self.header.source, length)?;
        if length != 0 && !frame_type.allows_data() {
            return Err(FrameError::UnexpectedData {
                raw_type: frame_type.to_raw(),
                length,
            });
        }
        if length < 2 && frame_type.is_proprietary() {
            return Err(FrameError::MissingVendorId {
                raw_type: frame_type.to_raw(),
                length,
            });
        }
        Ok(length)
    }
}
