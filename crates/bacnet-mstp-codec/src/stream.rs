//! Caller-buffered incremental decoding.

use crate::frame::{
    decode_extended_in_place_fields, ExtendedFrame, FrameType, MAX_EXTENDED_LENGTH,
    MIN_EXTENDED_LENGTH,
};
use crate::{decode_frame, is_cobs_raw, Frame, FrameError, HEADER_LENGTH, PREAMBLE};

/// Result of feeding one or more octets to [`StreamDecoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeEvent<'a> {
    /// The input does not yet contain a complete frame.
    NeedMore,
    /// A validated frame is available. It remains borrowed until the next push.
    Frame { frame: Frame<'a>, consumed: usize },
    /// A validated Extended DER or DNER frame is available.
    ExtendedFrame {
        /// Decoded frame borrowing the receive buffer.
        frame: ExtendedFrame<'a>,
        /// Number of wire octets consumed by this frame.
        consumed: usize,
    },
    /// Non-frame input was discarded while looking for the next preamble.
    ///
    /// This is synchronization noise, not a received-invalid-frame event in
    /// the Clause 9 receive state machine.
    Invalid { discard: usize },
    /// A candidate frame was malformed and discarded.
    Malformed { discard: usize, error: FrameError },
    /// The caller-owned receive buffer could not accept another octet.
    Error(FrameError),
}

/// Receive phase represented by the decoder's retained bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPhase {
    /// Searching for the first preamble octet; retained bytes are only noise.
    Sync,
    /// The first preamble octet has been recognized.
    Preamble,
    /// Both preamble octets are present and the fixed header is incomplete.
    Header,
    /// A valid fixed header is present and frame data remains incomplete.
    Data,
}

/// Incremental decoder backed by a caller-owned receive buffer.
pub struct StreamDecoder<'a> {
    storage: &'a mut [u8],
    length: usize,
    pending: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalEvent {
    NeedMore,
    Frame { consumed: usize },
    ExtendedFrame { consumed: usize },
    Invalid { discard: usize },
    Malformed { discard: usize, error: FrameError },
}

impl<'a> StreamDecoder<'a> {
    /// Create an empty decoder over `storage`.
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self {
            storage,
            length: 0,
            pending: 0,
        }
    }

    /// Number of buffered octets, including a frame awaiting the next push.
    pub const fn buffered_len(&self) -> usize {
        self.length
    }

    /// Capacity of the caller-owned receive buffer.
    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return the semantic receive phase, excluding retained resync noise.
    pub fn phase(&self) -> StreamPhase {
        if self.pending != 0 || self.length == 0 || self.storage[0] != PREAMBLE[0] {
            return StreamPhase::Sync;
        }
        if self.length == 1 {
            return StreamPhase::Preamble;
        }
        if self.storage[1] != PREAMBLE[1] {
            return StreamPhase::Sync;
        }
        if self.length < 8 {
            StreamPhase::Header
        } else {
            StreamPhase::Data
        }
    }

    /// Clear all buffered input and pending frame state.
    pub fn reset(&mut self) {
        self.length = 0;
        self.pending = 0;
    }

    /// Feed one octet and return the first resulting decode event.
    pub fn push(&mut self, byte: u8) -> DecodeEvent<'_> {
        self.push_slice(core::slice::from_ref(&byte))
    }

    /// Feed a DMA or serial slice and return the first resulting event.
    pub fn push_slice(&mut self, input: &[u8]) -> DecodeEvent<'_> {
        self.commit_pending();
        if input.len() > self.storage.len().saturating_sub(self.length) {
            return DecodeEvent::Error(FrameError::BufferExhausted {
                needed: self.length + input.len(),
                available: self.storage.len(),
            });
        }
        let end = self.length + input.len();
        self.storage[self.length..end].copy_from_slice(input);
        self.length = end;
        match self.try_decode() {
            InternalEvent::NeedMore => DecodeEvent::NeedMore,
            InternalEvent::Frame { consumed } => self.frame_event(consumed),
            InternalEvent::ExtendedFrame { consumed } => self.frame_event(consumed),
            InternalEvent::Invalid { discard } => DecodeEvent::Invalid { discard },
            InternalEvent::Malformed { discard, error } => {
                DecodeEvent::Malformed { discard, error }
            }
        }
    }

    fn commit_pending(&mut self) {
        if self.pending == 0 {
            return;
        }
        let consumed = self.pending.min(self.length);
        let remaining = self.length - consumed;
        self.storage.copy_within(consumed..self.length, 0);
        self.length = remaining;
        self.pending = 0;
    }

    fn frame_event(&mut self, consumed: usize) -> DecodeEvent<'_> {
        if self.length >= 3 && is_cobs_raw(self.storage[2]) {
            let discard = self.extended_candidate_len();
            let result = {
                let storage = &mut self.storage[..self.length];
                decode_extended_in_place_fields(storage)
            };
            match result {
                Ok((layout, decoded_length)) => {
                    self.pending = consumed;
                    let frame = ExtendedFrame {
                        header: crate::frame::FrameHeader::new(
                            FrameType::from_raw(layout.raw_type),
                            layout.destination,
                            layout.source,
                            decoded_length,
                        ),
                        data: &self.storage
                            [layout.encoded_data_start..layout.encoded_data_start + decoded_length],
                    };
                    return DecodeEvent::ExtendedFrame { frame, consumed };
                }
                Err(error) => {
                    self.discard(discard);
                    return DecodeEvent::Malformed { discard, error };
                }
            }
        }
        match decode_frame(&self.storage[..self.length]) {
            Ok((frame, _)) => {
                self.pending = consumed;
                DecodeEvent::Frame { frame, consumed }
            }
            Err(error) => DecodeEvent::Error(error),
        }
    }

    fn try_decode(&mut self) -> InternalEvent {
        if self.length == 0 {
            return InternalEvent::NeedMore;
        }
        if self.storage[0] != PREAMBLE[0] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        if self.length < 2 {
            return InternalEvent::NeedMore;
        }
        if self.storage[1] != PREAMBLE[1] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        if self.length >= 3 && is_cobs_raw(self.storage[2]) {
            if self.length < 8 {
                return InternalEvent::NeedMore;
            }
            if !crate::crc8_valid(&self.storage[2..8]) {
                self.discard(1);
                return InternalEvent::Malformed {
                    discard: 1,
                    error: FrameError::HeaderCrcMismatch,
                };
            }
            let length = u16::from_be_bytes([self.storage[5], self.storage[6]]) as usize;
            if !(MIN_EXTENDED_LENGTH..=MAX_EXTENDED_LENGTH).contains(&length) {
                self.discard(1);
                return InternalEvent::Malformed {
                    discard: 1,
                    error: FrameError::ExtendedLengthOutOfRange {
                        length,
                        minimum: MIN_EXTENDED_LENGTH,
                        maximum: MAX_EXTENDED_LENGTH,
                    },
                };
            }
            let total = PREAMBLE.len() + HEADER_LENGTH + length + 2;
            if self.length < total {
                return InternalEvent::NeedMore;
            }
            return InternalEvent::ExtendedFrame { consumed: total };
        }
        match decode_frame(&self.storage[..self.length]) {
            Ok((_, consumed)) => {
                self.pending = consumed;
                InternalEvent::Frame { consumed }
            }
            Err(FrameError::TooShort { .. } | FrameError::Truncated { .. }) => {
                InternalEvent::NeedMore
            }
            Err(error @ FrameError::DataCrcMismatch { data_length }) => {
                let discard = if self.length >= 8 {
                    8 + data_length + 2
                } else {
                    1
                };
                self.discard(discard.min(self.length));
                InternalEvent::Malformed { discard, error }
            }
            Err(error) => {
                self.discard(1);
                InternalEvent::Malformed { discard: 1, error }
            }
        }
    }

    fn extended_candidate_len(&self) -> usize {
        if self.length < PREAMBLE.len() + HEADER_LENGTH || !crate::crc8_valid(&self.storage[2..8]) {
            return 1;
        }
        let length = u16::from_be_bytes([self.storage[5], self.storage[6]]) as usize;
        if !(MIN_EXTENDED_LENGTH..=MAX_EXTENDED_LENGTH).contains(&length) {
            return 1;
        }
        (PREAMBLE.len() + HEADER_LENGTH + length + 2).min(self.length)
    }

    fn discard(&mut self, count: usize) {
        let count = count.min(self.length);
        let remaining = self.length - count;
        self.storage.copy_within(count..self.length, 0);
        self.length = remaining;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{
        encode_extended_into, FrameHeader, FrameType, MAX_EXTENDED_FRAME_LENGTH,
        MIN_EXTENDED_MPDU_DATA,
    };
    use crate::BROADCAST_MAC;

    fn extended_wire() -> ([u8; MAX_EXTENDED_FRAME_LENGTH], usize) {
        let payload = [0xA5; MIN_EXTENDED_MPDU_DATA];
        let header = FrameHeader::new(
            FrameType::ExtendedDataNotExpectingReply,
            BROADCAST_MAC,
            1,
            payload.len(),
        );
        let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH];
        let used = encode_extended_into(&mut wire, &header, &payload).unwrap();
        (wire, used)
    }

    #[test]
    fn every_extended_stream_split_point_reassembles() {
        let (wire, used) = extended_wire();
        for split in 0..=used {
            let mut storage = [0u8; MAX_EXTENDED_FRAME_LENGTH];
            let mut decoder = StreamDecoder::new(&mut storage);
            let event = if split == used {
                decoder.push_slice(&wire[..split])
            } else {
                assert!(matches!(
                    decoder.push_slice(&wire[..split]),
                    DecodeEvent::NeedMore
                ));
                decoder.push_slice(&wire[split..])
            };
            assert!(matches!(
                event,
                DecodeEvent::ExtendedFrame { consumed, .. } if consumed == used
            ));
        }
    }

    #[test]
    fn corrupted_extended_frame_is_discarded_and_resynchronizes() {
        let (mut wire, used) = extended_wire();
        wire[used - 1] ^= 1;
        let mut storage = [0u8; MAX_EXTENDED_FRAME_LENGTH];
        let mut decoder = StreamDecoder::new(&mut storage);
        assert!(matches!(
            decoder.push_slice(&wire[..used]),
            DecodeEvent::Malformed {
                discard,
                error: FrameError::ExtendedCrcMismatch
            } if discard == used
        ));
        let (valid, valid_len) = extended_wire();
        assert!(matches!(
            decoder.push_slice(&valid[..valid_len]),
            DecodeEvent::ExtendedFrame { .. }
        ));
    }

    #[test]
    fn mixed_standard_and_extended_frames_are_streamed_in_order() {
        let (extended, extended_len) = extended_wire();
        let standard = Frame::new(FrameType::Token, 0, 7, &[]);
        let mut wire = [0u8; MAX_EXTENDED_FRAME_LENGTH + 8];
        let standard_len = standard.encode_into(&mut wire).unwrap();
        wire[standard_len..standard_len + extended_len].copy_from_slice(&extended[..extended_len]);
        let mut storage = [0u8; MAX_EXTENDED_FRAME_LENGTH + 8];
        let mut decoder = StreamDecoder::new(&mut storage);
        assert!(matches!(
            decoder.push_slice(&wire[..standard_len + extended_len]),
            DecodeEvent::Frame { consumed, .. } if consumed == standard_len
        ));
        assert!(matches!(
            decoder.push_slice(&[]),
            DecodeEvent::ExtendedFrame { consumed, .. } if consumed == extended_len
        ));
    }
}
