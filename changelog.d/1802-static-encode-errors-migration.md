---
section: Migration notes
---
- **Encoding errors (Rust API, #1802):** match `Error::Encode(EncodeError)`,
  not `Error::Encoding`, for codec encoder refusals, and `Error::Range`, not
  `Error::OutOfRange`, for their range checks. `Error::Segmentation` holds a
  `SegmentationError` enum, also without `alloc`. New: `Error::encode`,
  `Error::range` and `Error::into_encode_refusal`.
