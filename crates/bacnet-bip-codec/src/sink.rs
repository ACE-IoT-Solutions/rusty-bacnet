//! Byte sinks for [`Bvll::encode_to_sink`](crate::Bvll::encode_to_sink).

use crate::BvllError;

/// A destination for encoded BVLL octets.
///
/// [`Bvll::encode_to_sink`](crate::Bvll::encode_to_sink) asks for
/// [`remaining_capacity`](BvllSink::remaining_capacity) first. A sink that
/// reports one is never handed a message that does not fit, so the fixed
/// sinks in this crate never hold part of a message. A sink that reports
/// `None` can grow, and is written without that check: if one of its writes
/// fails after an earlier one succeeded, the earlier octets stay in it, and
/// the caller has to discard them.
pub trait BvllSink {
    /// Octets of room left, or `None` when the sink grows as needed.
    fn remaining_capacity(&self) -> Option<usize>;

    /// Append `bytes`, or fail without appending any of them.
    fn write(&mut self, bytes: &[u8]) -> Result<(), BvllError>;
}

/// Writes at the front of the slice and advances it past what was written,
/// so after a successful encode the slice holds the unused tail.
impl BvllSink for &mut [u8] {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), BvllError> {
        if self.len() < bytes.len() {
            return Err(BvllError::BufferExhausted {
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
impl<const N: usize> BvllSink for heapless::Vec<u8, N> {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(N - self.len())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), BvllError> {
        let available = N - self.len();
        self.extend_from_slice(bytes)
            .map_err(|_| BvllError::BufferExhausted {
                needed: bytes.len(),
                available,
            })
    }
}

#[cfg(feature = "alloc")]
impl BvllSink for alloc::vec::Vec<u8> {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), BvllError> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

#[cfg(feature = "bytes")]
impl BvllSink for bytes::BytesMut {
    fn remaining_capacity(&self) -> Option<usize> {
        None
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), BvllError> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}
