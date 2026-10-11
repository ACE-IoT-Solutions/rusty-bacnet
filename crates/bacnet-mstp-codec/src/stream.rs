//! Incremental decoding over caller-owned storage.

use crate::frame::{wire_length, DATA_OFFSET};
use crate::{
    decode_extended_frame_in_place, decode_frame, ExtendedFrame, Frame, FrameError, FrameHeader,
    FrameType, PREAMBLE,
};

/// The outcome of one [`StreamDecoder::push_slice`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeEvent<'a> {
    /// Nothing complete and nothing to discard: push more octets.
    NeedMore,
    /// A valid frame. It borrows the decoder's storage and stays buffered
    /// until the next push, which removes it.
    Frame {
        /// The decoded frame.
        frame: Frame<'a>,
        /// The frame's wire length.
        consumed: usize,
    },
    /// A valid Extended DER or DNER frame with its COBS envelope removed.
    ExtendedFrame {
        /// The decoded frame.
        frame: ExtendedFrame<'a>,
        /// The encoded frame's wire length.
        consumed: usize,
    },
    /// Line noise was discarded while searching for a preamble.
    ///
    /// This is not a received invalid frame in terms of the Clause 9.5.4
    /// receive state machine.
    Invalid {
        /// Octets discarded.
        discard: usize,
    },
    /// A candidate frame failed a check and was dropped.
    Malformed {
        /// Octets discarded. For a bad data CRC this is the whole frame; for
        /// anything else it is the first preamble octet, and the octets after it
        /// are scanned again.
        discard: usize,
        /// Why the candidate was dropped.
        error: FrameError,
        /// The candidate's destination address once its header CRC has
        /// passed, including for a header refused afterwards (source 255, a
        /// length over the limit). `None` when the header CRC failed or the
        /// header never completed. See [`StreamDecoder`] for why a receiver
        /// needs it.
        destination: Option<u8>,
    },
    /// The pushed slice did not fit in the free storage. None of it was
    /// buffered; see [`StreamDecoder`].
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

/// Incremental frame decoder over a buffer the caller lends it.
///
/// Feed it received octets with [`push`](Self::push) or
/// [`push_slice`](Self::push_slice). Each call buffers the input and reports
/// at most one [`DecodeEvent`], so:
///
/// - A [`DecodeEvent::Frame`] borrows the storage. The frame stays buffered
///   until the next push, which commits it (drops it from the buffer) before
///   it reads new input. An empty push commits it too.
/// - After any event other than [`DecodeEvent::NeedMore`], more may already be
///   buffered, such as a second frame from the same slice or more noise. Call
///   `push_slice(&[])` until it returns `NeedMore` to drain it.
/// - A slice longer than the free storage is refused whole with
///   [`DecodeEvent::Error`] carrying [`FrameError::BufferExhausted`] (the
///   slice's length and the free room), and nothing is buffered. Drain, then
///   push it again or in smaller pieces.
///
/// Size the storage for the largest frame you accept:
/// [`MAX_STANDARD_FRAME_LENGTH`](crate::MAX_STANDARD_FRAME_LENGTH) covers every
/// standard frame, plus room for the slices you push. A candidate that could
/// never fit is reported once, as [`DecodeEvent::Malformed`] with
/// [`FrameError::FrameTooLong`], as soon as its header shows its size (or at
/// its first octet, if the storage cannot even hold a header). Its first
/// octet is discarded, the decoder rescans from the next one, and its
/// remaining octets drain as noise. Clause 9.5.4.4 has a receiver skip such a
/// frame's data by count instead; that belongs to a receive state machine.
///
/// The decoder does not know which station it serves, so it reports every
/// candidate it drops, whatever the destination. A receive state machine
/// (Clause 9.5.4.4) only counts an oversized, COBS-encoded or bad-data-CRC
/// frame as invalid when it is addressed to that station or broadcast, and
/// skips one meant for another station without complaint. Such a caller
/// should check the `destination` of a [`DecodeEvent::Malformed`] carrying
/// [`FrameError::FrameTooLong`], [`FrameError::UnsupportedCobsFrame`] or
/// [`FrameError::DataCrcMismatch`] before treating it as an invalid frame. A
/// bad header (CRC, source 255, length over the limit) counts wherever it was
/// going.
///
/// That includes a COBS-type frame whose Length falls outside the range COBS
/// frames allow, or whose source is 255: Clause 9.5.8 rejects its header.
/// The decoder doesn't tell those apart yet. It refuses every COBS type with
/// [`FrameError::UnsupportedCobsFrame`] before it looks at the source or the
/// length, so a receiver sees such a header as a good one and skips it when
/// it is for another station.
///
/// The decoder keeps no time. A receiver that aborts a partial frame after a
/// silence (Clause 9.5.4) calls [`reset`](Self::reset).
///
/// ```
/// use bacnet_mstp_codec::{DecodeEvent, Frame, FrameType, StreamDecoder};
///
/// let mut wire = [0u8; 16];
/// Frame::new(FrameType::Token, 1, 0, &[]).encode_into(&mut wire[..8])?;
/// Frame::new(FrameType::Token, 2, 1, &[]).encode_into(&mut wire[8..])?;
///
/// let mut storage = [0u8; 64];
/// let mut decoder = StreamDecoder::new(&mut storage);
/// let mut destinations = Vec::new();
/// let mut event = decoder.push_slice(&wire);
/// loop {
///     match event {
///         DecodeEvent::NeedMore => break,
///         DecodeEvent::Frame { frame, .. } => destinations.push(frame.header.destination),
///         _ => {}
///     }
///     // Commit the frame, if any, and look for the next one.
///     event = decoder.push_slice(&[]);
/// }
/// assert_eq!(destinations, [1, 2]);
/// assert_eq!(decoder.buffered_len(), 0);
/// # Ok::<(), bacnet_mstp_codec::FrameError>(())
/// ```
pub struct StreamDecoder<'a> {
    storage: &'a mut [u8],
    length: usize,
    pending: usize,
}

/// Offset of the destination octet: after the preamble and the type octet.
const DESTINATION_INDEX: usize = PREAMBLE.len() + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalEvent {
    NeedMore,
    Frame {
        consumed: usize,
    },
    ExtendedFrame {
        consumed: usize,
        decoded_length: usize,
    },
    Invalid {
        discard: usize,
    },
    Malformed {
        discard: usize,
        error: FrameError,
        destination: Option<u8>,
    },
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
        if self.length < DATA_OFFSET {
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

    /// Feed one octet; see [`push_slice`](Self::push_slice).
    pub fn push(&mut self, byte: u8) -> DecodeEvent<'_> {
        self.push_slice(core::slice::from_ref(&byte))
    }

    /// Commit a frame delivered by the previous push, buffer `input`, and
    /// report the first resulting event. The [type docs](Self) describe the
    /// protocol.
    pub fn push_slice(&mut self, input: &[u8]) -> DecodeEvent<'_> {
        self.commit_pending();
        let free = self.storage.len() - self.length;
        if input.len() > free {
            return DecodeEvent::Error(FrameError::BufferExhausted {
                needed: input.len(),
                available: free,
            });
        }
        let end = self.length + input.len();
        self.storage[self.length..end].copy_from_slice(input);
        self.length = end;
        match self.try_decode() {
            InternalEvent::NeedMore => DecodeEvent::NeedMore,
            InternalEvent::Frame { consumed } => self.frame_event(consumed),
            InternalEvent::ExtendedFrame {
                consumed,
                decoded_length,
            } => self.extended_frame_event(consumed, decoded_length),
            InternalEvent::Invalid { discard } => DecodeEvent::Invalid { discard },
            InternalEvent::Malformed {
                discard,
                error,
                destination,
            } => DecodeEvent::Malformed {
                discard,
                error,
                destination,
            },
        }
    }

    fn commit_pending(&mut self) {
        if self.pending == 0 {
            return;
        }
        self.discard(self.pending);
        self.pending = 0;
    }

    fn frame_event(&self, consumed: usize) -> DecodeEvent<'_> {
        // try_decode just accepted these octets; decoding them again only
        // re-borrows the frame from storage, so this cannot fail.
        match decode_frame(&self.storage[..self.length]) {
            Ok((frame, _)) => DecodeEvent::Frame { frame, consumed },
            Err(error) => DecodeEvent::Error(error),
        }
    }

    fn extended_frame_event(&self, consumed: usize, decoded_length: usize) -> DecodeEvent<'_> {
        let frame = ExtendedFrame {
            header: FrameHeader::new(
                FrameType::from_raw(self.storage[2]),
                self.storage[3],
                self.storage[4],
            ),
            data: &self.storage[DATA_OFFSET..DATA_OFFSET + decoded_length],
        };
        DecodeEvent::ExtendedFrame { frame, consumed }
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
            return self.need_more(DATA_OFFSET, None);
        }
        if self.storage[1] != PREAMBLE[1] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        if self.length >= DATA_OFFSET && matches!(self.storage[2], 0x20 | 0x21) {
            return self.try_decode_extended();
        }
        let error = match decode_frame(&self.storage[..self.length]) {
            Ok((_, consumed)) => {
                self.pending = consumed;
                return InternalEvent::Frame { consumed };
            }
            Err(error) => error,
        };
        // Errors past the header CRC come with the whole header buffered,
        // so its destination octet can be trusted. Every variant is listed,
        // so a new error has to say which side of the CRC it falls on.
        let destination = match error {
            FrameError::TooShort { .. }
            | FrameError::InvalidPreamble { .. }
            | FrameError::HeaderCrcMismatch => None,
            FrameError::UnsupportedCobsFrame { .. }
            | FrameError::BroadcastSource
            | FrameError::Oversize { .. }
            | FrameError::Truncated { .. }
            | FrameError::DataCrcMismatch { .. } => Some(self.storage[DESTINATION_INDEX]),
            // decode_frame never returns these: they come from encoding or
            // from the stream decoder itself.
            FrameError::UnexpectedData { .. }
            | FrameError::MissingVendorId { .. }
            | FrameError::FrameTooLong { .. }
            | FrameError::BufferExhausted { .. }
            | FrameError::ExtendedLengthOutOfRange { .. }
            | FrameError::ExtendedTruncated { .. }
            | FrameError::ExtendedPayloadOutOfRange { .. }
            | FrameError::ExtendedCobs { .. }
            | FrameError::ExtendedCrcLength { .. }
            | FrameError::ExtendedCrcMismatch => None,
        };
        match error {
            FrameError::TooShort { needed, .. } => self.need_more(needed, None),
            FrameError::Truncated { data_length, .. } => {
                self.need_more(wire_length(data_length), destination)
            }
            FrameError::DataCrcMismatch { data_length } => {
                let discard = wire_length(data_length);
                self.discard(discard);
                InternalEvent::Malformed {
                    discard,
                    error,
                    destination,
                }
            }
            _ => {
                self.discard(1);
                InternalEvent::Malformed {
                    discard: 1,
                    error,
                    destination,
                }
            }
        }
    }

    fn try_decode_extended(&mut self) -> InternalEvent {
        match decode_extended_frame_in_place(&mut self.storage[..self.length]) {
            Ok((frame, consumed)) => {
                let decoded_length = frame.data.len();
                self.pending = consumed;
                InternalEvent::ExtendedFrame {
                    consumed,
                    decoded_length,
                }
            }
            Err(FrameError::ExtendedTruncated { needed, .. }) => {
                let destination = Some(self.storage[DESTINATION_INDEX]);
                self.need_more(needed, destination)
            }
            Err(error) => {
                let destination = match error {
                    FrameError::TooShort { .. }
                    | FrameError::InvalidPreamble { .. }
                    | FrameError::HeaderCrcMismatch => None,
                    _ => Some(self.storage[DESTINATION_INDEX]),
                };
                self.discard(1);
                InternalEvent::Malformed {
                    discard: 1,
                    error,
                    destination,
                }
            }
        }
    }

    /// Wait for more input, unless the candidate needs `needed` octets in all
    /// and that is more than the storage holds: then drop its first octet so
    /// the decoder resynchronizes instead of filling up for good.
    fn need_more(&mut self, needed: usize, destination: Option<u8>) -> InternalEvent {
        let capacity = self.storage.len();
        if needed <= capacity {
            return InternalEvent::NeedMore;
        }
        self.discard(1);
        InternalEvent::Malformed {
            discard: 1,
            error: FrameError::FrameTooLong { needed, capacity },
            destination,
        }
    }

    fn discard(&mut self, count: usize) {
        let count = count.min(self.length);
        self.storage.copy_within(count..self.length, 0);
        self.length -= count;
    }
}
