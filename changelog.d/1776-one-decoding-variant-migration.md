---
section: Migration notes
---
- **Decoding errors (Rust API, #1776):** `Error::DecodingMessage` and
  `DecodeMessage` are removed; match `Error::Decoding(DecodeError)`, now
  `#[non_exhaustive]` with an item `index`. Oversized lighting, color and
  elevator command members fail with `Error::Range`, not `Error::OutOfRange`,
  and the WPM cursor error holds `error: DecodeError` instead of `message`.
