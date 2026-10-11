//! MS/TP frame compatibility API backed by `bacnet-mstp-codec`.
//!
//! This module preserves the transport crate's historical owned-`Bytes` API while
//! delegating wire encoding, decoding, CRCs, and frame classification to the
//! allocation-free codec crate.

use bacnet_mstp_codec as codec;
use bacnet_mstp_codec::FrameSink;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

pub use codec::{
    crc16, crc16_accumulate_all, crc16_valid, crc8, crc8_accumulate_all, crc8_valid, find_preamble,
    FrameType, BROADCAST_MAC, DATA_CRC_RESIDUAL, HEADER_CRC_RESIDUAL, HEADER_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_MASTER, MAX_MPDU_DATA,
    MAX_STANDARD_MPDU_DATA, MIN_EXTENDED_LENGTH, PREAMBLE,
};

pub(crate) const MAX_STANDARD_FRAME_LENGTH: usize = codec::MAX_STANDARD_FRAME_LENGTH;

/// A decoded MS/TP frame.
///
/// The transport API historically owned its payload in [`Bytes`]. The underlying
/// codec borrows payloads, so the adapter copies only after a successful decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MstpFrame {
    pub frame_type: FrameType,
    pub destination: u8,
    pub source: u8,
    /// NPDU/data payload (empty for Token, PFM, ReplyToPFM, ReplyPostponed).
    pub data: Bytes,
}

/// Encode an MS/TP frame into the buffer.
///
/// The legacy encoder did not validate source addresses and accepted payloads on
/// control frame types; constructing the codec header from the actual payload
/// preserves both behaviors.
pub fn encode_frame(buf: &mut BytesMut, frame: &MstpFrame) -> Result<(), Error> {
    if frame.frame_type.is_cobs() {
        // Keep the historical BytesMut API while delegating the extended wire
        // format to the caller-buffered codec.  Encoding into a temporary first
        // preserves the standard API's all-or-nothing behavior on errors.
        let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH];
        let used = codec::encode_extended_into(
            &mut wire,
            &codec::FrameHeader::new(
                frame.frame_type,
                frame.destination,
                frame.source,
                frame.data.len(),
            ),
            &frame.data,
        )
        .map_err(Error::from)?;
        buf.extend_from_slice(&wire[..used]);
        return Ok(());
    }
    let borrowed = codec::Frame::new(
        frame.frame_type,
        frame.destination,
        frame.source,
        &frame.data,
    );
    buf.encode(borrowed).map_err(Error::from)?;
    Ok(())
}

/// Result of incremental/streaming MS/TP frame decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDecode {
    /// A complete, validated frame was decoded.
    Complete { frame: MstpFrame, consumed: usize },
    /// More bytes are required before a decode decision can be made.
    NeedMore,
    /// The buffer contains invalid data; discard at least `discard` bytes and resync.
    Invalid { discard: usize },
}

/// Incrementally decode an MS/TP frame from a receive buffer (starting at the preamble).
pub fn decode_frame_stream(data: &[u8]) -> StreamDecode {
    let mut scratch = [0u8; MAX_EXTENDED_MPDU_DATA];
    decode_frame_stream_with_scratch(data, &mut scratch)
}

/// Incrementally decode a frame using caller-owned decoded-payload storage.
///
/// The returned [`MstpFrame`] owns a copy for compatibility with the hosted
/// transport API; `scratch` is nevertheless required for extended COBS decode
/// so a caller can bound and account for the transformed payload workspace.
pub fn decode_frame_stream_with_scratch(data: &[u8], scratch: &mut [u8]) -> StreamDecode {
    if data.is_empty() {
        return StreamDecode::NeedMore;
    }
    if data[0] != PREAMBLE[0] {
        return StreamDecode::Invalid { discard: 1 };
    }
    if data.len() < 2 {
        return StreamDecode::NeedMore;
    }
    if data[1] != PREAMBLE[1] {
        return StreamDecode::Invalid { discard: 1 };
    }
    if data.len() < 2 + HEADER_LENGTH {
        return StreamDecode::NeedMore;
    }

    if codec::is_cobs_raw(data[2]) {
        if !matches!(
            data[2],
            value if value == FrameType::ExtendedDataExpectingReply.to_raw()
                || value == FrameType::ExtendedDataNotExpectingReply.to_raw()
        ) {
            return StreamDecode::Invalid { discard: 1 };
        }
        return match codec::decode_extended_frame(data, scratch) {
            Ok((frame, consumed)) => StreamDecode::Complete {
                frame: owned_extended_frame(frame),
                consumed,
            },
            Err(
                codec::FrameError::TooShort { .. } | codec::FrameError::ExtendedTruncated { .. },
            ) => StreamDecode::NeedMore,
            Err(_) => StreamDecode::Invalid {
                discard: extended_candidate_length(data),
            },
        };
    }

    match codec::decode_frame(data) {
        Ok((frame, consumed)) => StreamDecode::Complete {
            frame: owned_frame(frame),
            consumed,
        },
        Err(codec::FrameError::TooShort { .. } | codec::FrameError::Truncated { .. }) => {
            StreamDecode::NeedMore
        }
        Err(codec::FrameError::DataCrcMismatch { data_length }) => StreamDecode::Invalid {
            discard: 2 + HEADER_LENGTH + data_length + 2,
        },
        Err(_) => StreamDecode::Invalid { discard: 1 },
    }
}

/// When no full preamble is found, retain a trailing lone `0x55` for the next chunk.
pub fn retain_lone_preamble_byte(buf: &mut Vec<u8>) {
    let mut length = buf.len();
    codec::retain_lone_preamble_byte(buf.as_mut_slice(), &mut length);
    buf.truncate(length);
}

/// Decode an MS/TP frame from raw bytes (starting at the preamble).
pub fn decode_frame(data: &[u8]) -> Result<(MstpFrame, usize), Error> {
    if data.len() >= 3 && codec::is_cobs_raw(data[2]) {
        let mut scratch = [0u8; MAX_EXTENDED_MPDU_DATA];
        let (frame, consumed) =
            codec::decode_extended_frame(data, &mut scratch).map_err(Error::from)?;
        return Ok((owned_extended_frame(frame), consumed));
    }
    let (frame, consumed) = codec::decode_frame(data).map_err(Error::from)?;
    Ok((owned_frame(frame), consumed))
}

fn owned_frame(frame: codec::Frame<'_>) -> MstpFrame {
    MstpFrame {
        frame_type: frame.header.frame_type,
        destination: frame.header.destination,
        source: frame.header.source,
        data: Bytes::copy_from_slice(frame.data),
    }
}

fn owned_extended_frame(frame: codec::ExtendedFrame<'_>) -> MstpFrame {
    MstpFrame {
        frame_type: frame.header.frame_type,
        destination: frame.header.destination,
        source: frame.header.source,
        data: Bytes::copy_from_slice(frame.data),
    }
}

fn extended_candidate_length(data: &[u8]) -> usize {
    if data.len() < 2 + HEADER_LENGTH || !codec::crc8_valid(&data[2..8]) {
        return 1;
    }
    let length = u16::from_be_bytes([data[5], data[6]]) as usize;
    if !(codec::MIN_EXTENDED_LENGTH..=codec::MAX_EXTENDED_LENGTH).contains(&length) {
        return 1;
    }
    (2 + HEADER_LENGTH + length + 2).min(data.len())
}

#[cfg(test)]
#[path = "mstp_frame_tests.rs"]
mod tests;
