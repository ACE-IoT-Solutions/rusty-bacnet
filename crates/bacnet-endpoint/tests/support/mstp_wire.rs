//! An owned MS/TP frame for test peers, encoded and decoded by the codec
//! the transport re-exports (`bacnet_transport::mstp::codec`).

#![allow(dead_code)]

use bacnet_transport::mstp::codec;
pub use bacnet_transport::mstp::codec::FrameType;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

/// A decoded frame that owns its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MstpFrame {
    /// Kind of frame.
    pub frame_type: FrameType,
    /// Destination station address (`0xFF` is broadcast).
    pub destination: u8,
    /// Source station address.
    pub source: u8,
    /// Data field.
    pub data: Bytes,
}

/// Decode one frame from the start of `wire`, with the octets it took.
pub fn decode_frame(wire: &[u8]) -> Result<(MstpFrame, usize), Error> {
    let (frame, consumed) =
        codec::decode_frame(wire).map_err(|error| Error::Encoding(error.to_string()))?;
    Ok((
        MstpFrame {
            frame_type: frame.header.frame_type,
            destination: frame.header.destination,
            source: frame.header.source,
            data: Bytes::copy_from_slice(frame.data),
        },
        consumed,
    ))
}

/// Append `frame`'s wire octets to `buf`.
pub fn encode_frame(buf: &mut BytesMut, frame: &MstpFrame) -> Result<(), Error> {
    let mut wire = Vec::new();
    codec::Frame::new(
        frame.frame_type,
        frame.destination,
        frame.source,
        &frame.data,
    )
    .encode_to_sink(&mut wire)
    .map_err(|error| Error::Encoding(error.to_string()))?;
    buf.extend_from_slice(&wire);
    Ok(())
}

fn empty(frame_type: FrameType, destination: u8, source: u8) -> MstpFrame {
    MstpFrame {
        frame_type,
        destination,
        source,
        data: Bytes::new(),
    }
}

/// A Token from `source` to `destination`.
pub fn token_frame(destination: u8, source: u8) -> MstpFrame {
    empty(FrameType::Token, destination, source)
}

/// A Poll For Master from `source` to `destination`.
pub fn pfm_frame(destination: u8, source: u8) -> MstpFrame {
    empty(FrameType::PollForMaster, destination, source)
}

/// A Reply To Poll For Master from `source` to `destination`.
pub fn reply_to_poll(destination: u8, source: u8) -> MstpFrame {
    empty(FrameType::ReplyToPollForMaster, destination, source)
}
