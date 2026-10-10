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
    FrameType, BROADCAST_MAC, DATA_CRC_RESIDUAL, HEADER_CRC_RESIDUAL, HEADER_LENGTH, MAX_MASTER,
    MAX_MPDU_DATA, MAX_STANDARD_MPDU_DATA, PREAMBLE,
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

#[cfg(test)]
#[path = "mstp_frame_legacy.rs"]
mod legacy;

#[cfg(test)]
#[path = "mstp_frame_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mstp_frame_differential_tests.rs"]
mod differential_tests;
