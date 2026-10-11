//! A small, allocation-free BACnet MS/TP frame codec.
//!
//! The wire codec is usable without an allocator.  [`Frame`] borrows its
//! payload, [`Frame::encode_into`] writes into caller-owned storage, and
//! [`StreamDecoder`] keeps its receive state in a caller-owned byte buffer.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod cobs;
mod crc;
mod error;
mod frame;
mod sink;
mod stream;

pub use cobs::{
    cobs_decode_xor55, cobs_decode_xor55_in_place, cobs_encode_xor55, cobs_encoded_len, CobsError,
};
pub use crc::{
    crc16, crc16_accumulate_all, crc16_valid, crc32k, crc32k_accumulate_all, crc32k_valid, crc8,
    crc8_accumulate_all, crc8_valid,
};
pub use error::FrameError;
pub use frame::{
    decode_frame, encode_into, find_preamble, is_cobs_raw, is_valid_source,
    retain_lone_preamble_byte, Frame, FrameHeader, FrameType,
};
pub use sink::{encode_to_sink, FrameSink};
pub use stream::{DecodeEvent, StreamDecoder, StreamPhase};

/// Compatibility alias for callers that name stream outcomes `StreamEvent`.
pub type StreamEvent<'a> = DecodeEvent<'a>;
/// Compatibility alias for the former transport stream result name.
pub type StreamDecode<'a> = DecodeEvent<'a>;

/// MS/TP preamble bytes.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];
/// Header length after the preamble, including the header CRC.
pub const HEADER_LENGTH: usize = 6;
/// Maximum NPDU data length in a standard MS/TP frame.
pub const MAX_STANDARD_MPDU_DATA: usize = 501;
/// Compatibility name for [`MAX_STANDARD_MPDU_DATA`].
pub const MAX_MPDU_DATA: usize = MAX_STANDARD_MPDU_DATA;
/// Maximum master station address.
pub const MAX_MASTER: u8 = 127;
/// Broadcast MAC spelling on MS/TP.
pub const BROADCAST_MAC: u8 = 0xFF;
/// Good header CRC receiver residual.
pub const HEADER_CRC_RESIDUAL: u8 = 0x55;
/// Good data CRC receiver residual.
pub const DATA_CRC_RESIDUAL: u16 = 0xF0B8;
/// Good Extended MS/TP CRC-32K receiver residual.
pub const CRC32K_RESIDUAL: u32 = 0x0843_323B;
/// Maximum encoded length of a standard frame.
pub const MAX_STANDARD_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_STANDARD_MPDU_DATA + 2;
