//! Typed failures emitted by the frame codec.

use core::fmt;

use crate::cobs::CobsError;

/// The field in an Extended MS/TP frame where COBS validation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedField {
    /// The encoded NPDU field.
    Data,
    /// The five-octet encoded CRC field.
    Crc,
}

/// A frame could not be encoded or decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The input ended before the requested bytes were available.
    TooShort { needed: usize, available: usize },
    /// The two preamble octets were not `0x55, 0xFF`.
    InvalidPreamble { first: u8, second: u8 },
    /// The header CRC did not match.
    HeaderCrcMismatch,
    /// A raw type in the reserved COBS/CRC-32K range was received.
    UnsupportedCobsFrame { raw_type: u8 },
    /// The source address is not a valid MS/TP master address.
    InvalidSource { source: u8 },
    /// The data length is above the standard-frame limit.
    Oversize { length: usize, maximum: usize },
    /// An encoder was asked to write an unsupported COBS frame type.
    EncodeUnsupportedCobsFrame { raw_type: u8 },
    /// An encoder was asked to write an oversized payload.
    EncodeOversize { length: usize, maximum: usize },
    /// The header's declared data length disagrees with the borrowed payload.
    HeaderLengthMismatch { declared: usize, actual: usize },
    /// The frame ended before its data and CRC were complete.
    Truncated {
        data_length: usize,
        available: usize,
    },
    /// The data CRC did not match.
    DataCrcMismatch { data_length: usize },
    /// The destination storage could not hold the encoded frame.
    BufferExhausted { needed: usize, available: usize },
    /// An Extended MS/TP Length value is outside the generic COBS envelope.
    ExtendedLengthOutOfRange {
        /// Header Length value, excluding the compatibility two-octet offset.
        length: usize,
        /// Smallest permitted header Length.
        minimum: usize,
        /// Largest permitted header Length.
        maximum: usize,
    },
    /// An extended frame ended before its encoded fields were complete.
    ExtendedTruncated { needed: usize, available: usize },
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
    ExtendedCrcLength { length: usize },
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
            Self::EncodeUnsupportedCobsFrame { raw_type } => {
                write!(f, "unsupported COBS frame type {raw_type}")
            }
            Self::InvalidSource { source } => write!(f, "invalid source address {source:#04X}"),
            Self::Oversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::EncodeOversize { length, maximum } => {
                write!(f, "data length {length} exceeds maximum {maximum}")
            }
            Self::HeaderLengthMismatch { declared, actual } => {
                write!(
                    f,
                    "header length {declared} does not match payload length {actual}"
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
                write!(f, "extended {:?} COBS error: {error}", field)
            }
            Self::ExtendedCrcLength { length } => {
                write!(f, "extended CRC decoded to {length} octets, expected 4")
            }
            Self::ExtendedCrcMismatch => f.write_str("extended CRC-32K mismatch"),
        }
    }
}

#[cfg(feature = "alloc")]
impl From<FrameError> for bacnet_types::error::Error {
    fn from(error: FrameError) -> Self {
        use alloc::format;
        match error {
            FrameError::TooShort { .. } => Self::decoding(0, "MS/TP frame too short"),
            FrameError::InvalidPreamble { first, second } => Self::decoding(
                0,
                format!("MS/TP expected preamble 0x55 0xFF, got 0x{first:02X} 0x{second:02X}"),
            ),
            FrameError::HeaderCrcMismatch => Self::decoding(7, "MS/TP header CRC mismatch"),
            FrameError::UnsupportedCobsFrame { raw_type } => Self::decoding(
                2,
                format!("MS/TP COBS-encoded frame type {raw_type} is unsupported"),
            ),
            FrameError::EncodeUnsupportedCobsFrame { raw_type } => Self::Encoding(format!(
                "MS/TP COBS-encoded frame type {raw_type} is unsupported"
            )),
            FrameError::InvalidSource { source: 0xFF } => {
                Self::decoding(4, "MS/TP source address cannot be broadcast (0xFF)")
            }
            FrameError::InvalidSource { source } => Self::decoding(
                4,
                format!("MS/TP source address 0x{source:02X} exceeds MAX_MASTER (127)"),
            ),
            FrameError::Oversize { length, maximum } => Self::decoding(
                5,
                format!("MS/TP data length {length} exceeds maximum {maximum}"),
            ),
            FrameError::EncodeOversize { length, maximum } => Self::Encoding(format!(
                "MS/TP data length {length} exceeds maximum {maximum}"
            )),
            FrameError::HeaderLengthMismatch { declared, actual } => Self::Encoding(format!(
                "MS/TP header data length {declared} does not match payload length {actual}"
            )),
            FrameError::Truncated {
                data_length,
                available,
            } => Self::decoding(
                8,
                format!(
                    "MS/TP frame truncated: need {} bytes for data+CRC, have {available}",
                    data_length + 2
                ),
            ),
            FrameError::DataCrcMismatch { data_length } => {
                Self::decoding(8 + data_length, "MS/TP data CRC mismatch")
            }
            FrameError::BufferExhausted { needed, available } => {
                Self::buffer_too_short(needed, available)
            }
            FrameError::ExtendedLengthOutOfRange {
                length,
                minimum,
                maximum,
            } => Self::decoding(
                5,
                format!("MS/TP extended Length {length} outside {minimum}..={maximum}"),
            ),
            FrameError::ExtendedTruncated { needed, available } => Self::decoding(
                8,
                format!(
                    "MS/TP extended frame truncated: need {needed} encoded octets, have {available}"
                ),
            ),
            FrameError::ExtendedPayloadOutOfRange {
                length,
                minimum,
                maximum,
            } => Self::decoding(
                8,
                format!("MS/TP extended payload length {length} outside {minimum}..={maximum}"),
            ),
            FrameError::ExtendedCobs { field, error } => {
                Self::decoding(8, format!("MS/TP extended {field:?} COBS error: {error}"))
            }
            FrameError::ExtendedCrcLength { length } => Self::decoding(
                8,
                format!("MS/TP extended CRC decoded to {length} octets, expected 4"),
            ),
            FrameError::ExtendedCrcMismatch => Self::decoding(8, "MS/TP extended CRC-32K mismatch"),
        }
    }
}
