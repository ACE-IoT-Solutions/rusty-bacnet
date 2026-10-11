//! Incremental decoding over caller-owned storage.

use crate::frame::{wire_length, DATA_OFFSET};
use crate::{decode_frame, Frame, FrameError, PREAMBLE};

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
///   [`DecodeEvent::Error`] and nothing is buffered. Drain, then push it again
///   or in smaller pieces.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalEvent {
    NeedMore,
    Frame { consumed: usize },
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
        if input.len() > self.storage.len() - self.length {
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

    fn try_decode(&mut self) -> InternalEvent {
        if self.length == 0 {
            return InternalEvent::NeedMore;
        }
        if self.storage[0] != PREAMBLE[0] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        if self.length < 2 {
            return self.need_more(DATA_OFFSET);
        }
        if self.storage[1] != PREAMBLE[1] {
            self.discard(1);
            return InternalEvent::Invalid { discard: 1 };
        }
        match decode_frame(&self.storage[..self.length]) {
            Ok((_, consumed)) => {
                self.pending = consumed;
                InternalEvent::Frame { consumed }
            }
            Err(FrameError::TooShort { needed, .. }) => self.need_more(needed),
            Err(FrameError::Truncated { data_length, .. }) => {
                self.need_more(wire_length(data_length))
            }
            Err(error @ FrameError::DataCrcMismatch { data_length }) => {
                let discard = wire_length(data_length);
                self.discard(discard);
                InternalEvent::Malformed { discard, error }
            }
            Err(error) => {
                self.discard(1);
                InternalEvent::Malformed { discard: 1, error }
            }
        }
    }

    /// Wait for more input, unless the candidate needs `needed` octets in all
    /// and that is more than the storage holds: then drop its first octet so
    /// the decoder resynchronizes instead of filling up for good.
    fn need_more(&mut self, needed: usize) -> InternalEvent {
        let capacity = self.storage.len();
        if needed <= capacity {
            return InternalEvent::NeedMore;
        }
        self.discard(1);
        InternalEvent::Malformed {
            discard: 1,
            error: FrameError::FrameTooLong { needed, capacity },
        }
    }

    fn discard(&mut self, count: usize) {
        let count = count.min(self.length);
        self.storage.copy_within(count..self.length, 0);
        self.length -= count;
    }
}
