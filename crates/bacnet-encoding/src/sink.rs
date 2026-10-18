//! Append-only output for the sink-based encoders.
//!
//! The tag-header functions in [`crate::tags`] and bounded scalar encoders in
//! [`crate::primitives`] support these sinks. Other encoder families still use
//! their existing buffers. The crate still needs either its `std` or its
//! `alloc` feature, including with a bounded sink.

use core::fmt;

/// Storage that accepts a complete byte slice or leaves its output unchanged.
///
/// A write appends after earlier output. On failure, neither the output bytes
/// nor the output position may change. Implementers must uphold that contract;
/// it is not a partial-I/O interface.
///
/// The tag-header and bounded scalar encoders check any advertised capacity,
/// then write their complete output once. They leave no partial header or
/// scalar on failure. This does not promise rollback across several encoder
/// calls: a custom sink can refuse a later write even after a total-capacity
/// check, leaving earlier writes in place. A future multi-write encoder must
/// state its own failure behavior.
///
/// Implemented for `&mut [u8]`, [`alloc::vec::Vec<u8>`], [`bytes::BytesMut`], and
/// `heapless::Vec<u8, N>` with the `heapless` feature. Growable adapters may
/// allocate and retain their ordinary allocation-failure behavior.
///
/// ```
/// use bacnet_encoding::sink::EncodeSink;
///
/// let mut storage = [0xA5; 4];
/// let mut tail = &mut storage[..];
/// tail.write(&[1, 2])?;
/// assert_eq!(tail.remaining_capacity(), Some(2));
/// assert_eq!(tail.write(&[3, 4, 5]).unwrap_err().available, 2);
/// tail.write(&[3, 4])?;
/// assert!(tail.is_empty());
/// assert_eq!(storage, [1, 2, 3, 4]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub trait EncodeSink {
    /// Remaining append space, or `None` when no fixed bound is advertised.
    ///
    /// Growable containers return `None`, not their current allocation's spare
    /// capacity. A sink may still reject a write; the encoder passes through
    /// its [`SinkFull`] without changing the error.
    fn remaining_capacity(&self) -> Option<usize>;

    /// Append all of `bytes`, or fail without changing output bytes or position.
    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>;
}

/// A byte append did not fit its sink.
///
/// This error holds no allocated message. It does not describe allocation
/// failure in a growable container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkFull {
    /// Number of bytes in the refused append.
    pub needed: usize,
    /// Available append space when it was refused.
    pub available: usize,
}

impl fmt::Display for SinkFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sink full: need {}, have {}",
            self.needed, self.available
        )
    }
}

impl core::error::Error for SinkFull {}

/// Writes at the front and advances the slice to its unused tail.
/// A failed write leaves both the slice and its backing bytes unchanged.
impl EncodeSink for &mut [u8] {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        if bytes.len() > self.len() {
            return Err(SinkFull {
                needed: bytes.len(),
                available: self.len(),
            });
        }
        let (head, tail) = core::mem::take(self).split_at_mut(bytes.len());
        head.copy_from_slice(bytes);
        *self = tail;
        Ok(())
    }
}

#[cfg(feature = "heapless")]
impl<const N: usize> EncodeSink for heapless::Vec<u8, N> {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(N - self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        let available = N - self.len();
        self.extend_from_slice(bytes).map_err(|_| SinkFull {
            needed: bytes.len(),
            available,
        })
    }
}

impl EncodeSink for alloc::vec::Vec<u8> {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

impl EncodeSink for bytes::BytesMut {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}
