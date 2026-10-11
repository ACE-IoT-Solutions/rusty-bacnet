//! COBS/XOR-0x55 primitives used by Extended MS/TP frames.
//!
//! The Extended MS/TP wire format applies ordinary Consistent Overhead Byte
//! Stuffing to each field and then XORs every resulting octet with `0x55`.
//! These routines deliberately do not append a COBS delimiter: fields are
//! length-delimited by the surrounding frame format.

use core::fmt;

/// Failure returned by a COBS/XOR-0x55 operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CobsError {
    /// The encoded input contains no code octet.
    EmptyInput,
    /// A code octet decoded to zero.
    InvalidCode {
        /// Offset of the invalid code octet in the encoded field.
        offset: usize,
    },
    /// A code octet claims bytes beyond the end of the field.
    CodeExceedsInput {
        /// Offset of the code octet in the encoded field.
        offset: usize,
        /// Decoded code value.
        code: u8,
        /// Number of encoded data octets remaining after the code.
        available: usize,
    },
    /// The caller's output storage cannot hold the complete result.
    OutputExhausted {
        /// Exact output size required by the operation.
        needed: usize,
        /// Caller-provided output capacity.
        available: usize,
    },
}

impl fmt::Display for CobsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => f.write_str("COBS input is empty"),
            Self::InvalidCode { offset } => write!(f, "invalid COBS code at offset {offset}"),
            Self::CodeExceedsInput {
                offset,
                code,
                available,
            } => write!(
                f,
                "COBS code {code} at offset {offset} exceeds {available} available bytes"
            ),
            Self::OutputExhausted { needed, available } => {
                write!(f, "COBS output exhausted: need {needed}, have {available}")
            }
        }
    }
}

/// Return the exact number of bytes produced by [`cobs_encode_xor55`].
pub const fn cobs_encoded_len(input: &[u8]) -> usize {
    let mut read = 0;
    let mut nonzero = 0;
    let mut codes = 0;
    while read < input.len() {
        codes += 1;
        let mut run = 0;
        while read < input.len() && run < 254 && input[read] != 0 {
            read += 1;
            run += 1;
            nonzero += 1;
        }
        if read < input.len() && input[read] == 0 {
            read += 1;
            if run == 254 && read < input.len() {
                // A full block cannot itself signal the zero that follows
                // it, so reserve a standalone code-1 block for that zero.
                codes += 1;
            }
            // A trailing zero is represented by a final empty block.  This
            // is real input, unlike the phantom block that must not follow a
            // non-zero field ending exactly at a 254-byte block boundary.
            if read == input.len() {
                codes += 1;
                if run == 254 {
                    // A full block does not imply a zero before the next
                    // code.  Keep a final empty code so a trailing zero is
                    // represented without making a full non-zero block grow
                    // a phantom zero.
                    codes += 1;
                }
            }
        }
    }
    if codes == 0 {
        1
    } else {
        nonzero + codes
    }
}

/// Encode one field using ordinary COBS followed by XOR with `0x55`.
///
/// The complete required capacity is checked before writing, so an exhausted
/// output buffer is an all-or-nothing error.
pub fn cobs_encode_xor55(input: &[u8], output: &mut [u8]) -> Result<usize, CobsError> {
    let needed = cobs_encoded_len(input);
    if output.len() < needed {
        return Err(CobsError::OutputExhausted {
            needed,
            available: output.len(),
        });
    }

    if input.is_empty() {
        output[0] = 0x01 ^ 0x55;
        return Ok(1);
    }

    let mut read = 0;
    let mut write = 0;
    while read < input.len() {
        let code_offset = write;
        write += 1;
        let mut code = 1u8;
        while read < input.len() && code != 0xFF && input[read] != 0 {
            output[write] = input[read] ^ 0x55;
            write += 1;
            read += 1;
            code += 1;
        }
        output[code_offset] = code ^ 0x55;
        if read < input.len() && input[read] == 0 {
            read += 1;
            if code == 0xFF && read < input.len() {
                output[write] = 0x01 ^ 0x55;
                write += 1;
            }
            if read == input.len() {
                output[write] = 0x01 ^ 0x55;
                write += 1;
                if code == 0xFF {
                    output[write] = 0x01 ^ 0x55;
                    write += 1;
                }
                break;
            }
        }
    }
    Ok(write)
}

/// Validate an XOR-0x55 COBS field and return its decoded length.
fn decoded_len(input: &[u8]) -> Result<usize, CobsError> {
    if input.is_empty() {
        return Err(CobsError::EmptyInput);
    }
    let mut read = 0;
    let mut needed = 0;
    while read < input.len() {
        let offset = read;
        let code = input[read] ^ 0x55;
        if code == 0 {
            return Err(CobsError::InvalidCode { offset });
        }
        read += 1;
        let available = input.len() - read;
        let data_len = code as usize - 1;
        if data_len > available {
            return Err(CobsError::CodeExceedsInput {
                offset,
                code,
                available,
            });
        }
        needed += data_len;
        read += data_len;
        if read < input.len() && code != 0xFF {
            needed += 1;
        }
    }
    Ok(needed)
}

/// Decode one XOR-0x55 COBS field into caller-owned storage.
///
/// Validation and capacity checks complete before any output byte is written.
pub fn cobs_decode_xor55(input: &[u8], output: &mut [u8]) -> Result<usize, CobsError> {
    let needed = decoded_len(input)?;
    if output.len() < needed {
        return Err(CobsError::OutputExhausted {
            needed,
            available: output.len(),
        });
    }
    decode_validated(input, output);
    Ok(needed)
}

/// Decode an XOR-0x55 COBS field in place.
pub fn cobs_decode_xor55_in_place(buffer: &mut [u8]) -> Result<usize, CobsError> {
    let needed = decoded_len(buffer)?;
    // COBS decoding never expands a field, so each write is at or behind its
    // read cursor. Copying backwards is unnecessary and would complicate the
    // zero-insertion boundary.
    let mut read = 0;
    let mut write = 0;
    while read < buffer.len() {
        let code = buffer[read] ^ 0x55;
        read += 1;
        let data_len = code as usize - 1;
        for _ in 0..data_len {
            buffer[write] = buffer[read] ^ 0x55;
            write += 1;
            read += 1;
        }
        if read < buffer.len() && code != 0xFF {
            buffer[write] = 0;
            write += 1;
        }
    }
    debug_assert_eq!(write, needed);
    Ok(needed)
}

fn decode_validated(input: &[u8], output: &mut [u8]) {
    let mut read = 0;
    let mut write = 0;
    while read < input.len() {
        let code = input[read] ^ 0x55;
        read += 1;
        let data_len = code as usize - 1;
        for _ in 0..data_len {
            output[write] = input[read] ^ 0x55;
            write += 1;
            read += 1;
        }
        if read < input.len() && code != 0xFF {
            output[write] = 0;
            write += 1;
        }
    }
}
