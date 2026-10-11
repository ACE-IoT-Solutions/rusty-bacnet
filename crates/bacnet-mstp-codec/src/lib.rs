//! An allocation-free codec for standard BACnet MS/TP frames (Clause 9).
//!
//! The crate covers the wire format and nothing above it: frame types, the
//! header and data CRCs, one-shot encode and decode, and an incremental decoder
//! for a serial byte stream. Token passing, timers and line control belong to
//! the layer that uses it.
//!
//! The default build never allocates. [`Frame`] borrows its data,
//! [`Frame::encode_into`] writes into caller storage, and [`StreamDecoder`]
//! keeps its receive state in a buffer the caller lends it. Only the optional
//! `alloc` and `bytes` sinks grow on the heap.
//!
//! # Validation
//!
//! Decoding is a raw, link-level check. [`decode_frame`] verifies the preamble,
//! both CRCs and the length limit, and refuses a source address of 255, which
//! no station may send from (Clause 9.3). It accepts every other source,
//! including the slave-only range 128-254: whether a station may use its
//! address in its role is for the receiver to judge. It also accepts data on a
//! frame type that never carries any, and a proprietary frame too short for
//! its vendor identifier, so a receiver can see what arrived.
//!
//! Encoding is strict. [`Frame::encode_into`] and [`Frame::encode_to_sink`]
//! refuse source 255, data on the four frame types defined without any (see
//! [`FrameType::allows_data`]), and a proprietary frame (128-255) with less
//! than its 2-octet vendor identifier (Clause 9.3.11). The reserved types 8-31
//! get the same CRC, source and length checks as any frame in both
//! directions, but the codec doesn't police their content.
//!
//! Both directions refuse the COBS-encoded frame types 32-127, which this
//! crate does not implement yet.
//!
//! # Features
//!
//! No feature needs `std`. Each adds a [`FrameSink`] implementation:
//!
//! - `alloc`: for `alloc::vec::Vec<u8>`.
//! - `bytes`: for `bytes::BytesMut`; implies `alloc`.
//! - `heapless`: for `heapless::Vec<u8, N>`.
//!
//! # Example
//!
//! ```
//! use bacnet_mstp_codec::{decode_frame, Frame, FrameType, MAX_STANDARD_FRAME_LENGTH};
//!
//! let frame = Frame::new(FrameType::BACnetDataNotExpectingReply, 0x10, 0x05, &[0x01, 0x00]);
//! let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
//! let length = frame.encode_into(&mut wire)?;
//!
//! let (decoded, consumed) = decode_frame(&wire[..length])?;
//! assert_eq!(decoded, frame);
//! assert_eq!(consumed, length);
//! # Ok::<(), bacnet_mstp_codec::FrameError>(())
//! ```

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod crc;
mod error;
mod frame;
mod sink;
mod stream;

pub use crc::{crc16, crc16_accumulate_all, crc16_valid, crc8, crc8_accumulate_all, crc8_valid};
pub use error::FrameError;
pub use frame::{decode_frame, find_preamble, Frame, FrameHeader, FrameType};
pub use sink::FrameSink;
pub use stream::{DecodeEvent, StreamDecoder, StreamPhase};

/// The two preamble octets that start every frame.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];
/// Header length after the preamble: type, destination, source, the two
/// length octets and the header CRC.
pub const HEADER_LENGTH: usize = 6;
/// Largest data field a standard (non-COBS) frame can carry.
pub const MAX_STANDARD_MPDU_DATA: usize = 501;
/// Highest station address a master may use. Slaves may also use 128-254.
pub const MAX_MASTER: u8 = 127;
/// The broadcast address. It is valid only as a destination.
pub const BROADCAST_MAC: u8 = 0xFF;
/// What the receiver's header CRC register holds after a good header.
pub const HEADER_CRC_RESIDUAL: u8 = 0x55;
/// What the receiver's data CRC register holds after good data.
pub const DATA_CRC_RESIDUAL: u16 = 0xF0B8;
/// Longest standard frame on the wire: preamble, header, the largest data
/// field and its CRC.
pub const MAX_STANDARD_FRAME_LENGTH: usize =
    PREAMBLE.len() + HEADER_LENGTH + MAX_STANDARD_MPDU_DATA + 2;
