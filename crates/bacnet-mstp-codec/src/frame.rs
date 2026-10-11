//! Frame headers and one-shot encode/decode operations.

use crate::error::ExtendedField;
use crate::{
    cobs_decode_xor55, cobs_decode_xor55_in_place, cobs_encode_xor55, cobs_encoded_len, crc16,
    crc16_valid, crc32k, crc8, crc8_valid, FrameError, BROADCAST_MAC, HEADER_LENGTH,
    MAX_STANDARD_MPDU_DATA, PREAMBLE,
};

/// Smallest decoded NPDU accepted by a BACnet Extended MS/TP frame.
pub const MIN_EXTENDED_MPDU_DATA: usize = 502;
/// Largest decoded NPDU accepted by a BACnet Extended MS/TP frame.
pub const MAX_EXTENDED_MPDU_DATA: usize = 1497;
/// Smallest generic COBS frame Length value accepted by this codec.
pub const MIN_EXTENDED_LENGTH: usize = 5;
/// Largest generic COBS frame Length value accepted by this codec.
pub const MAX_EXTENDED_LENGTH: usize = 1506;
/// Number of encoded octets occupied by an Extended MS/TP CRC field.
pub const EXTENDED_CRC_LENGTH: usize = 5;
/// Largest encoded Extended MS/TP data field.
pub const MAX_EXTENDED_ENCODED_DATA: usize = MAX_EXTENDED_LENGTH - 3;
/// Largest complete Extended MS/TP frame without optional padding.
pub const MAX_EXTENDED_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_EXTENDED_LENGTH + 2;

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
    /// BACnet Extended Data Expecting Reply frame.
    ExtendedDataExpectingReply = 0x20,
    /// BACnet Extended Data Not Expecting Reply frame.
    ExtendedDataNotExpectingReply = 0x21,
    /// A currently unassigned standard frame type.
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
            0x20 => Self::ExtendedDataExpectingReply,
            0x21 => Self::ExtendedDataNotExpectingReply,
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
            Self::ExtendedDataExpectingReply => 0x20,
            Self::ExtendedDataNotExpectingReply => 0x21,
            Self::Unknown(value) => value,
        }
    }

    /// Whether this frame type carries a data payload.
    pub const fn has_data(self) -> bool {
        matches!(
            self,
            Self::TestRequest
                | Self::TestResponse
                | Self::BACnetDataExpectingReply
                | Self::BACnetDataNotExpectingReply
                | Self::ExtendedDataExpectingReply
                | Self::ExtendedDataNotExpectingReply
        )
    }

    /// Whether this raw type belongs to the reserved COBS/CRC-32K range.
    pub const fn is_cobs(self) -> bool {
        let raw = self.to_raw();
        raw >= 0x20 && raw <= 0x7F
    }
}

/// The allocation-free header fields of an MS/TP frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Frame type.
    pub frame_type: FrameType,
    /// Destination station address.
    pub destination: u8,
    /// Source station address.
    pub source: u8,
    /// Number of data bytes following the header.
    pub data_length: usize,
}

impl FrameHeader {
    /// Construct a header for a payload of `data_length` bytes.
    pub const fn new(
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data_length: usize,
    ) -> Self {
        Self {
            frame_type,
            destination,
            source,
            data_length,
        }
    }
}

/// A decoded frame borrowing its data from the input or stream buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// Borrowed NPDU or test payload.
    pub data: &'a [u8],
}

/// A decoded BACnet DER or DNER frame with its COBS envelope removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedFrame<'a> {
    /// Header fields. `data_length` is the decoded NPDU length.
    pub header: FrameHeader,
    /// Borrowed, decoded NPDU bytes.
    pub data: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Construct a borrowed frame and derive its header length from `data`.
    pub const fn new(frame_type: FrameType, destination: u8, source: u8, data: &'a [u8]) -> Self {
        Self {
            header: FrameHeader::new(frame_type, destination, source, data.len()),
            data,
        }
    }

    /// Encode this frame into caller-owned storage.
    pub fn encode_into(self, output: &mut [u8]) -> Result<usize, FrameError> {
        encode_into(output, &self.header, self.data)
    }
}

/// Encode a frame into caller-owned storage, returning its wire length.
pub fn encode_into(
    output: &mut [u8],
    header: &FrameHeader,
    data: &[u8],
) -> Result<usize, FrameError> {
    let data_length = data.len();
    let raw_type = header.frame_type.to_raw();
    if (0x20..=0x7F).contains(&raw_type) {
        return Err(FrameError::EncodeUnsupportedCobsFrame { raw_type });
    }
    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::EncodeOversize {
            length: data_length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    if header.data_length != data_length {
        return Err(FrameError::HeaderLengthMismatch {
            declared: header.data_length,
            actual: data_length,
        });
    }
    let total = PREAMBLE.len() + HEADER_LENGTH + data_length + if data_length == 0 { 0 } else { 2 };
    if output.len() < total {
        return Err(FrameError::BufferExhausted {
            needed: total,
            available: output.len(),
        });
    }
    output[..2].copy_from_slice(&PREAMBLE);
    let fields = [
        raw_type,
        header.destination,
        header.source,
        (data_length >> 8) as u8,
        data_length as u8,
    ];
    output[2..7].copy_from_slice(&fields);
    output[7] = crc8(&fields);
    if data_length != 0 {
        output[8..8 + data_length].copy_from_slice(data);
        let crc = crc16(data).to_le_bytes();
        output[8 + data_length..10 + data_length].copy_from_slice(&crc);
    }
    Ok(total)
}

/// Decode one frame from bytes beginning with its preamble.
pub fn decode_frame(data: &[u8]) -> Result<(Frame<'_>, usize), FrameError> {
    if data.len() < 2 + HEADER_LENGTH {
        return Err(FrameError::TooShort {
            needed: 2 + HEADER_LENGTH,
            available: data.len(),
        });
    }
    if data[0] != PREAMBLE[0] || data[1] != PREAMBLE[1] {
        return Err(FrameError::InvalidPreamble {
            first: data[0],
            second: data[1],
        });
    }
    if !crc8_valid(&data[2..8]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let raw_type = data[2];
    if (0x20..=0x7F).contains(&raw_type) {
        return Err(FrameError::UnsupportedCobsFrame { raw_type });
    }
    let source = data[4];
    if source == BROADCAST_MAC {
        return Err(FrameError::InvalidSource { source });
    }
    let data_length = u16::from_be_bytes([data[5], data[6]]) as usize;
    if data_length > MAX_STANDARD_MPDU_DATA {
        return Err(FrameError::Oversize {
            length: data_length,
            maximum: MAX_STANDARD_MPDU_DATA,
        });
    }
    let body_start = 2 + HEADER_LENGTH;
    let total = body_start + if data_length == 0 { 0 } else { data_length + 2 };
    if data.len() < total {
        return Err(FrameError::Truncated {
            data_length,
            available: data.len() - body_start,
        });
    }
    if data_length != 0 && !crc16_valid(&data[body_start..total]) {
        return Err(FrameError::DataCrcMismatch { data_length });
    }
    let frame = Frame {
        header: FrameHeader::new(FrameType::from_raw(raw_type), data[3], source, data_length),
        data: &data[body_start..body_start + data_length],
    };
    Ok((frame, total))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExtendedLayout {
    pub(crate) raw_type: u8,
    pub(crate) destination: u8,
    pub(crate) source: u8,
    pub(crate) encoded_data_start: usize,
    pub(crate) encoded_data_length: usize,
    pub(crate) encoded_crc_start: usize,
    pub(crate) total: usize,
}

fn extended_layout(data: &[u8]) -> Result<ExtendedLayout, FrameError> {
    if data.len() < PREAMBLE.len() + HEADER_LENGTH {
        return Err(FrameError::TooShort {
            needed: PREAMBLE.len() + HEADER_LENGTH,
            available: data.len(),
        });
    }
    if data[0] != PREAMBLE[0] || data[1] != PREAMBLE[1] {
        return Err(FrameError::InvalidPreamble {
            first: data[0],
            second: data[1],
        });
    }
    if !crc8_valid(&data[2..8]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let raw_type = data[2];
    if !is_cobs_raw(raw_type) {
        return Err(FrameError::UnsupportedCobsFrame { raw_type });
    }
    let source = data[4];
    if source == BROADCAST_MAC {
        return Err(FrameError::InvalidSource { source });
    }
    let length = u16::from_be_bytes([data[5], data[6]]) as usize;
    if !(MIN_EXTENDED_LENGTH..=MAX_EXTENDED_LENGTH).contains(&length) {
        return Err(FrameError::ExtendedLengthOutOfRange {
            length,
            minimum: MIN_EXTENDED_LENGTH,
            maximum: MAX_EXTENDED_LENGTH,
        });
    }
    let total = PREAMBLE.len() + HEADER_LENGTH + length + 2;
    if data.len() < total {
        return Err(FrameError::ExtendedTruncated {
            needed: total,
            available: data.len(),
        });
    }
    let encoded_data_length = length - 3;
    let encoded_data_start = PREAMBLE.len() + HEADER_LENGTH;
    let encoded_crc_start = encoded_data_start + encoded_data_length;
    debug_assert_eq!(encoded_crc_start + EXTENDED_CRC_LENGTH, total);
    Ok(ExtendedLayout {
        raw_type,
        destination: data[3],
        source,
        encoded_data_start,
        encoded_data_length,
        encoded_crc_start,
        total,
    })
}

fn validate_extended_envelope(data: &[u8], layout: ExtendedLayout) -> Result<(), FrameError> {
    let encoded_data =
        &data[layout.encoded_data_start..layout.encoded_data_start + layout.encoded_data_length];
    let encoded_crc = &data[layout.encoded_crc_start..layout.total];
    let mut decoded_crc = [0u8; EXTENDED_CRC_LENGTH];
    let decoded_crc_length = cobs_decode_xor55(encoded_crc, &mut decoded_crc).map_err(|error| {
        FrameError::ExtendedCobs {
            field: ExtendedField::Crc,
            error,
        }
    })?;
    if decoded_crc_length != 4 {
        return Err(FrameError::ExtendedCrcLength {
            length: decoded_crc_length,
        });
    }
    if decoded_crc[..4] != crc32k(encoded_data).to_le_bytes() {
        return Err(FrameError::ExtendedCrcMismatch);
    }
    Ok(())
}

fn validate_extended_type(raw_type: u8) -> Result<(), FrameError> {
    if raw_type == FrameType::ExtendedDataExpectingReply.to_raw()
        || raw_type == FrameType::ExtendedDataNotExpectingReply.to_raw()
    {
        Ok(())
    } else {
        Err(FrameError::UnsupportedCobsFrame { raw_type })
    }
}

/// Encode one BACnet Extended DER or DNER frame into caller-owned storage.
pub fn encode_extended_into(
    output: &mut [u8],
    header: &FrameHeader,
    data: &[u8],
) -> Result<usize, FrameError> {
    let raw_type = header.frame_type.to_raw();
    validate_extended_type(raw_type)?;
    if data.len() < MIN_EXTENDED_MPDU_DATA || data.len() > MAX_EXTENDED_MPDU_DATA {
        return Err(FrameError::ExtendedPayloadOutOfRange {
            length: data.len(),
            minimum: MIN_EXTENDED_MPDU_DATA,
            maximum: MAX_EXTENDED_MPDU_DATA,
        });
    }
    if header.data_length != data.len() {
        return Err(FrameError::HeaderLengthMismatch {
            declared: header.data_length,
            actual: data.len(),
        });
    }
    let encoded_data_length = cobs_encoded_len(data);
    let encoded_crc_length = cobs_encoded_len(&[0u8; 4]);
    debug_assert_eq!(encoded_crc_length, EXTENDED_CRC_LENGTH);
    let length = encoded_data_length + encoded_crc_length - 2;
    debug_assert_eq!(length, encoded_data_length + 3);
    if length > MAX_EXTENDED_LENGTH {
        return Err(FrameError::ExtendedLengthOutOfRange {
            length,
            minimum: MIN_EXTENDED_LENGTH,
            maximum: MAX_EXTENDED_LENGTH,
        });
    }
    let total = PREAMBLE.len() + HEADER_LENGTH + encoded_data_length + encoded_crc_length;
    if output.len() < total {
        return Err(FrameError::BufferExhausted {
            needed: total,
            available: output.len(),
        });
    }
    output[..2].copy_from_slice(&PREAMBLE);
    let fields = [
        raw_type,
        header.destination,
        header.source,
        (length >> 8) as u8,
        length as u8,
    ];
    output[2..7].copy_from_slice(&fields);
    output[7] = crc8(&fields);
    let data_start = PREAMBLE.len() + HEADER_LENGTH;
    let data_end = data_start + encoded_data_length;
    let written = cobs_encode_xor55(data, &mut output[data_start..data_end]).map_err(|error| {
        FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        }
    })?;
    debug_assert_eq!(written, encoded_data_length);
    let crc_bytes = crc32k(&output[data_start..data_end]).to_le_bytes();
    let crc_written =
        cobs_encode_xor55(&crc_bytes, &mut output[data_end..total]).map_err(|error| {
            FrameError::ExtendedCobs {
                field: ExtendedField::Crc,
                error,
            }
        })?;
    debug_assert_eq!(crc_written, encoded_crc_length);
    Ok(total)
}

/// Decode a BACnet Extended DER or DNER frame into caller-owned scratch.
pub fn decode_extended_frame<'a>(
    data: &[u8],
    scratch: &'a mut [u8],
) -> Result<(ExtendedFrame<'a>, usize), FrameError> {
    let layout = extended_layout(data)?;
    validate_extended_envelope(data, layout)?;
    validate_extended_type(layout.raw_type)?;
    let encoded_data =
        &data[layout.encoded_data_start..layout.encoded_data_start + layout.encoded_data_length];
    let decoded_length =
        cobs_decode_xor55(encoded_data, scratch).map_err(|error| FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        })?;
    if !(MIN_EXTENDED_MPDU_DATA..=MAX_EXTENDED_MPDU_DATA).contains(&decoded_length) {
        return Err(FrameError::ExtendedPayloadOutOfRange {
            length: decoded_length,
            minimum: MIN_EXTENDED_MPDU_DATA,
            maximum: MAX_EXTENDED_MPDU_DATA,
        });
    }
    let frame = ExtendedFrame {
        header: FrameHeader::new(
            FrameType::from_raw(layout.raw_type),
            layout.destination,
            layout.source,
            decoded_length,
        ),
        data: &scratch[..decoded_length],
    };
    Ok((frame, layout.total))
}

/// Decode an Extended DER or DNER frame in place in a caller-owned buffer.
pub fn decode_extended_frame_in_place(
    data: &mut [u8],
) -> Result<(ExtendedFrame<'_>, usize), FrameError> {
    let (layout, decoded_length) = decode_extended_in_place_fields(data)?;
    let frame = ExtendedFrame {
        header: FrameHeader::new(
            FrameType::from_raw(layout.raw_type),
            layout.destination,
            layout.source,
            decoded_length,
        ),
        data: &data[layout.encoded_data_start..layout.encoded_data_start + decoded_length],
    };
    Ok((frame, layout.total))
}

pub(crate) fn decode_extended_in_place_fields(
    data: &mut [u8],
) -> Result<(ExtendedLayout, usize), FrameError> {
    let layout = extended_layout(data)?;
    validate_extended_envelope(data, layout)?;
    validate_extended_type(layout.raw_type)?;
    let data_end = layout.encoded_data_start + layout.encoded_data_length;
    let decoded_length = cobs_decode_xor55_in_place(&mut data[layout.encoded_data_start..data_end])
        .map_err(|error| FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        })?;
    if !(MIN_EXTENDED_MPDU_DATA..=MAX_EXTENDED_MPDU_DATA).contains(&decoded_length) {
        return Err(FrameError::ExtendedPayloadOutOfRange {
            length: decoded_length,
            minimum: MIN_EXTENDED_MPDU_DATA,
            maximum: MAX_EXTENDED_MPDU_DATA,
        });
    }
    Ok((layout, decoded_length))
}

/// Return the first offset containing the MS/TP preamble.
pub fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|pair| pair == PREAMBLE)
}

/// Keep a trailing first preamble octet while discarding other bytes.
pub fn retain_lone_preamble_byte(buffer: &mut [u8], length: &mut usize) {
    if *length != 0 && buffer[*length - 1] == PREAMBLE[0] {
        buffer[0] = PREAMBLE[0];
        *length = 1;
    } else {
        *length = 0;
    }
}

/// Return whether a raw frame type is reserved for COBS/CRC-32K framing.
pub const fn is_cobs_raw(raw: u8) -> bool {
    raw >= 0x20 && raw <= 0x7F
}

/// Any master or slave address can be a source; broadcast never can.
pub const fn is_valid_source(source: u8) -> bool {
    source != BROADCAST_MAC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_boundaries_round_trip_in_place() {
        for length in [MIN_EXTENDED_MPDU_DATA, MAX_EXTENDED_MPDU_DATA] {
            let payload = [0xA5; MAX_EXTENDED_MPDU_DATA];
            let header = FrameHeader::new(
                FrameType::ExtendedDataNotExpectingReply,
                BROADCAST_MAC,
                1,
                length,
            );
            let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH];
            let used = encode_extended_into(&mut wire, &header, &payload[..length]).unwrap();
            assert_eq!(
                used,
                PREAMBLE.len()
                    + HEADER_LENGTH
                    + cobs_encoded_len(&payload[..length])
                    + EXTENDED_CRC_LENGTH
            );
            let (frame, consumed) = decode_extended_frame_in_place(&mut wire[..used]).unwrap();
            assert_eq!(consumed, used);
            assert_eq!(
                frame.header.frame_type,
                FrameType::ExtendedDataNotExpectingReply
            );
            assert_eq!(frame.data, &payload[..length]);
        }
    }

    #[test]
    fn extended_rejects_standard_boundaries_and_bad_crc() {
        let payload = [0xA5; MAX_EXTENDED_MPDU_DATA + 1];
        let mut output = [0u8; MAX_EXTENDED_FRAME_LENGTH];
        for length in [MIN_EXTENDED_MPDU_DATA - 1, MAX_EXTENDED_MPDU_DATA + 1] {
            let header = FrameHeader::new(FrameType::ExtendedDataExpectingReply, 0, 1, length);
            assert!(matches!(
                encode_extended_into(&mut output, &header, &payload[..length]),
                Err(FrameError::ExtendedPayloadOutOfRange { .. })
            ));
        }
        let header = FrameHeader::new(
            FrameType::ExtendedDataExpectingReply,
            0,
            1,
            MIN_EXTENDED_MPDU_DATA,
        );
        let used =
            encode_extended_into(&mut output, &header, &payload[..MIN_EXTENDED_MPDU_DATA]).unwrap();
        output[8] ^= 1;
        assert_eq!(
            decode_extended_frame(&output[..used], &mut [0u8; MAX_EXTENDED_MPDU_DATA]),
            Err(FrameError::ExtendedCrcMismatch)
        );
    }

    #[test]
    fn only_der_and_dner_are_delivered_as_bacnet_extended_frames() {
        let payload = [0xA5; MIN_EXTENDED_MPDU_DATA];
        let header = FrameHeader::new(
            FrameType::ExtendedDataNotExpectingReply,
            BROADCAST_MAC,
            1,
            payload.len(),
        );
        let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH];
        let used = encode_extended_into(&mut wire, &header, &payload).unwrap();
        for raw_type in 0x22..=0x7f {
            wire[2] = raw_type;
            wire[7] = crc8(&wire[2..7]);
            assert_eq!(
                decode_extended_frame(&wire[..used], &mut [0u8; MAX_EXTENDED_MPDU_DATA]),
                Err(FrameError::UnsupportedCobsFrame { raw_type })
            );
        }
    }
}
