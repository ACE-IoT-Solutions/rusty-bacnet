---
section: Migration notes
---
- **NPDU decode errors (Rust API, #1770):** `NpduDecodeError` no longer
  converts from `bacnet_types::Error` or reports a `source()`; its
  `Malformed` case becomes `Error::Decoding`, and an empty or one-octet
  buffer is `Decoding` with kind `Missing` where it was `BufferTooShort`.
