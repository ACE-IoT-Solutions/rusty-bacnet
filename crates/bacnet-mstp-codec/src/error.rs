//! Typed failures emitted by the frame codec.

use core::fmt;

/// A frame could not be encoded or decoded.
///
/// Encode and decode share the variants that mean the same thing in both
/// directions; the caller knows which one it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Decode: the input ended before the header was complete.
    TooShort {
        /// Octets needed for the preamble and header.
        needed: usize,
        /// Octets supplied.
        available: usize,
    },
    /// Decode: the input does not start with the preamble `0x55, 0xFF`.
    InvalidPreamble {
        /// First input octet.
        first: u8,
        /// Second input octet.
        second: u8,
    },
    /// Decode: the header CRC does not match.
    HeaderCrcMismatch,
    /// A frame type in the COBS-encoded range 32-127, which this crate does
    /// not implement yet.
    UnsupportedCobsFrame {
        /// The frame type octet.
        raw_type: u8,
    },
    /// The source address is 255, the broadcast address, which no station may
    /// send from (Clause 9.3).
    BroadcastSource,
    /// The data field is longer than a standard frame allows.
    Oversize {
        /// Data length declared by the header (decode) or supplied (encode).
        length: usize,
        /// Largest standard data field, [`crate::MAX_STANDARD_MPDU_DATA`].
        maximum: usize,
    },
    /// Encode: data was supplied for a frame type that never carries any (see
    /// [`crate::FrameType::allows_data`]).
    UnexpectedData {
        /// The frame type octet.
        raw_type: u8,
        /// Length of the supplied data.
        length: usize,
    },
    /// Decode: the input ended before the data field and its CRC were complete.
    Truncated {
        /// Data length declared by the header.
        data_length: usize,
        /// Octets supplied after the header.
        available: usize,
    },
    /// Decode: the data CRC does not match.
    DataCrcMismatch {
        /// Data length declared by the header.
        data_length: usize,
    },
    /// Stream decode: a candidate frame is longer than the decoder's whole
    /// storage, so it can never be buffered. See [`crate::StreamDecoder`].
    FrameTooLong {
        /// Octets the candidate needs, from its preamble to its last CRC octet.
        needed: usize,
        /// The decoder's storage capacity.
        capacity: usize,
    },
    /// The destination had no room: an encode sink, or a stream decoder's free
    /// storage for a pushed slice.
    BufferExhausted {
        /// Octets needed.
        needed: usize,
        /// Octets of room.
        available: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { needed, available } => {
                write!(f, "frame too short: need {needed}, have {available}")
            }
            Self::InvalidPreamble { first, second } => {
                write!(f, "invalid preamble {first:#04X} {second:#04X}")
            }
            Self::HeaderCrcMismatch => f.write_str("header CRC mismatch"),
            Self::UnsupportedCobsFrame { raw_type } => {
                write!(f, "unsupported COBS frame type {raw_type}")
            }
            Self::BroadcastSource => f.write_str("source address is broadcast (0xFF)"),
            Self::Oversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::UnexpectedData { raw_type, length } => {
                write!(
                    f,
                    "frame type {raw_type} carries no data, got {length} octets"
                )
            }
            Self::Truncated {
                data_length,
                available,
            } => write!(
                f,
                "truncated data length {data_length}, available {available}"
            ),
            Self::DataCrcMismatch { .. } => f.write_str("data CRC mismatch"),
            Self::FrameTooLong { needed, capacity } => write!(
                f,
                "frame needs {needed} octets, decoder storage holds {capacity}"
            ),
            Self::BufferExhausted { needed, available } => {
                write!(f, "buffer exhausted: need {needed}, have {available}")
            }
        }
    }
}

impl core::error::Error for FrameError {}
