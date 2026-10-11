//! Typed failures emitted by the frame codec.

use core::fmt;

use crate::CobsError;

/// The field in an Extended MS/TP frame where COBS validation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedField {
    /// The encoded NPDU field.
    Data,
    /// The five-octet encoded CRC field.
    Crc,
}

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
    /// Encode: a proprietary frame (type 128-255) has fewer than the two data
    /// octets its vendor identifier takes (Clause 9.3.11).
    MissingVendorId {
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
    /// There was no room for the octets: an encode sink is too small for the
    /// frame, or a stream decoder's free storage is too small for a pushed
    /// slice. Nothing was stored.
    BufferExhausted {
        /// Octets this call had to store: the encoded frame, or the pushed
        /// slice.
        needed: usize,
        /// Free room at the time of the call: the sink's remaining capacity,
        /// or the decoder's capacity less what it already buffers.
        available: usize,
    },
    /// An Extended MS/TP Length value is outside the COBS envelope.
    ExtendedLengthOutOfRange {
        /// Header Length value, excluding the compatibility two-octet offset.
        length: usize,
        /// Smallest permitted header Length.
        minimum: usize,
        /// Largest permitted header Length.
        maximum: usize,
    },
    /// An extended frame ended before its encoded fields were complete.
    ExtendedTruncated {
        /// Complete encoded frame length needed.
        needed: usize,
        /// Encoded octets currently available.
        available: usize,
    },
    /// The decoded extended NPDU is outside the DER/DNER payload range.
    ExtendedPayloadOutOfRange {
        /// Decoded payload length.
        length: usize,
        /// Smallest supported decoded payload length.
        minimum: usize,
        /// Largest supported decoded payload length.
        maximum: usize,
    },
    /// A COBS field was malformed or could not fit caller-owned storage.
    ExtendedCobs {
        /// Field being decoded.
        field: ExtendedField,
        /// Detailed COBS failure.
        error: CobsError,
    },
    /// The encoded CRC field did not decode to exactly four octets.
    ExtendedCrcLength {
        /// Actual decoded CRC-field length.
        length: usize,
    },
    /// The CRC-32K over the encoded data did not match the encoded CRC field.
    ExtendedCrcMismatch,
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
            Self::MissingVendorId { raw_type, length } => write!(
                f,
                "proprietary frame type {raw_type} needs a 2-octet vendor ID, got {length} octets"
            ),
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
            Self::ExtendedLengthOutOfRange {
                length,
                minimum,
                maximum,
            } => write!(
                f,
                "extended frame Length {length} outside {minimum}..={maximum}"
            ),
            Self::ExtendedTruncated { needed, available } => write!(
                f,
                "truncated extended frame: need {needed} encoded octets, have {available}"
            ),
            Self::ExtendedPayloadOutOfRange {
                length,
                minimum,
                maximum,
            } => write!(
                f,
                "extended payload length {length} outside {minimum}..={maximum}"
            ),
            Self::ExtendedCobs { field, error } => {
                write!(f, "extended {field:?} COBS error: {error}")
            }
            Self::ExtendedCrcLength { length } => {
                write!(f, "extended CRC decoded to {length} octets, expected 4")
            }
            Self::ExtendedCrcMismatch => f.write_str("extended CRC-32K mismatch"),
        }
    }
}

impl core::error::Error for FrameError {}
