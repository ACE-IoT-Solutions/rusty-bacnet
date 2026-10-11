//! Explicit codec for the COBS/CRC-32K Extended MS/TP data frame profile.

use crate::{
    cobs_decode_xor55, cobs_decode_xor55_in_place, cobs_encode_xor55, cobs_encoded_len, crc32k,
    crc8, crc8_valid, ExtendedField, FrameError, FrameHeader, FrameType, BROADCAST_MAC,
    HEADER_LENGTH, PREAMBLE,
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

/// A decoded BACnet DER or DNER frame with its COBS envelope removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedFrame<'a> {
    /// Header fields.
    pub header: FrameHeader,
    /// Borrowed, decoded NPDU bytes.
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtendedLayout {
    raw_type: u8,
    destination: u8,
    source: u8,
    encoded_data_start: usize,
    encoded_data_length: usize,
    encoded_crc_start: usize,
    total: usize,
}

fn validate_extended_type(raw_type: u8) -> Result<(), FrameError> {
    if matches!(raw_type, 0x20 | 0x21) {
        Ok(())
    } else {
        Err(FrameError::UnsupportedCobsFrame { raw_type })
    }
}

fn extended_layout(data: &[u8]) -> Result<ExtendedLayout, FrameError> {
    let data_offset = PREAMBLE.len() + HEADER_LENGTH;
    if data.len() < data_offset {
        return Err(FrameError::TooShort {
            needed: data_offset,
            available: data.len(),
        });
    }
    if data[..PREAMBLE.len()] != PREAMBLE {
        return Err(FrameError::InvalidPreamble {
            first: data[0],
            second: data[1],
        });
    }
    if !crc8_valid(&data[PREAMBLE.len()..data_offset]) {
        return Err(FrameError::HeaderCrcMismatch);
    }
    let raw_type = data[2];
    validate_extended_type(raw_type)?;
    let source = data[4];
    if source == BROADCAST_MAC {
        return Err(FrameError::BroadcastSource);
    }
    let length = usize::from(u16::from_be_bytes([data[5], data[6]]));
    if !(MIN_EXTENDED_LENGTH..=MAX_EXTENDED_LENGTH).contains(&length) {
        return Err(FrameError::ExtendedLengthOutOfRange {
            length,
            minimum: MIN_EXTENDED_LENGTH,
            maximum: MAX_EXTENDED_LENGTH,
        });
    }
    let total = data_offset + length + 2;
    if data.len() < total {
        return Err(FrameError::ExtendedTruncated {
            needed: total,
            available: data.len(),
        });
    }
    let encoded_data_length = length - 3;
    let encoded_crc_start = data_offset + encoded_data_length;
    Ok(ExtendedLayout {
        raw_type,
        destination: data[3],
        source,
        encoded_data_start: data_offset,
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

/// Encode one BACnet Extended DER or DNER frame into caller-owned storage.
pub fn encode_extended_into(
    output: &mut [u8],
    header: &FrameHeader,
    data: &[u8],
) -> Result<usize, FrameError> {
    let raw_type = header.frame_type.to_raw();
    validate_extended_type(raw_type)?;
    if header.source == BROADCAST_MAC {
        return Err(FrameError::BroadcastSource);
    }
    if !(MIN_EXTENDED_MPDU_DATA..=MAX_EXTENDED_MPDU_DATA).contains(&data.len()) {
        return Err(FrameError::ExtendedPayloadOutOfRange {
            length: data.len(),
            minimum: MIN_EXTENDED_MPDU_DATA,
            maximum: MAX_EXTENDED_MPDU_DATA,
        });
    }
    let encoded_data_length = cobs_encoded_len(data);
    let length = encoded_data_length + EXTENDED_CRC_LENGTH - 2;
    if length > MAX_EXTENDED_LENGTH {
        return Err(FrameError::ExtendedLengthOutOfRange {
            length,
            minimum: MIN_EXTENDED_LENGTH,
            maximum: MAX_EXTENDED_LENGTH,
        });
    }
    let total = PREAMBLE.len() + HEADER_LENGTH + encoded_data_length + EXTENDED_CRC_LENGTH;
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
    cobs_encode_xor55(data, &mut output[data_start..data_end]).map_err(|error| {
        FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        }
    })?;
    let crc_bytes = crc32k(&output[data_start..data_end]).to_le_bytes();
    cobs_encode_xor55(&crc_bytes, &mut output[data_end..total]).map_err(|error| {
        FrameError::ExtendedCobs {
            field: ExtendedField::Crc,
            error,
        }
    })?;
    Ok(total)
}

/// Decode a BACnet Extended DER or DNER frame into caller-owned scratch.
pub fn decode_extended_frame<'a>(
    data: &[u8],
    scratch: &'a mut [u8],
) -> Result<(ExtendedFrame<'a>, usize), FrameError> {
    let layout = extended_layout(data)?;
    validate_extended_envelope(data, layout)?;
    let encoded_data =
        &data[layout.encoded_data_start..layout.encoded_data_start + layout.encoded_data_length];
    let decoded_length =
        cobs_decode_xor55(encoded_data, scratch).map_err(|error| FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        })?;
    validate_decoded_length(decoded_length)?;
    Ok((
        ExtendedFrame {
            header: FrameHeader::new(
                FrameType::from_raw(layout.raw_type),
                layout.destination,
                layout.source,
            ),
            data: &scratch[..decoded_length],
        },
        layout.total,
    ))
}

/// Decode an Extended DER or DNER frame in place in a caller-owned buffer.
pub fn decode_extended_frame_in_place(
    data: &mut [u8],
) -> Result<(ExtendedFrame<'_>, usize), FrameError> {
    let layout = extended_layout(data)?;
    validate_extended_envelope(data, layout)?;
    let data_end = layout.encoded_data_start + layout.encoded_data_length;
    let decoded_length = cobs_decode_xor55_in_place(&mut data[layout.encoded_data_start..data_end])
        .map_err(|error| FrameError::ExtendedCobs {
            field: ExtendedField::Data,
            error,
        })?;
    validate_decoded_length(decoded_length)?;
    Ok((
        ExtendedFrame {
            header: FrameHeader::new(
                FrameType::from_raw(layout.raw_type),
                layout.destination,
                layout.source,
            ),
            data: &data[layout.encoded_data_start..layout.encoded_data_start + decoded_length],
        },
        layout.total,
    ))
}

fn validate_decoded_length(length: usize) -> Result<(), FrameError> {
    if (MIN_EXTENDED_MPDU_DATA..=MAX_EXTENDED_MPDU_DATA).contains(&length) {
        Ok(())
    } else {
        Err(FrameError::ExtendedPayloadOutOfRange {
            length,
            minimum: MIN_EXTENDED_MPDU_DATA,
            maximum: MAX_EXTENDED_MPDU_DATA,
        })
    }
}
