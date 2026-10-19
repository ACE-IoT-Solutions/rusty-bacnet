---
section: Migration notes
---
- **Encoder refusals (Rust API, #1777):** `encode_apdu`, `encode_npdu`,
  `validate_max_segments`, `validate_max_apdu_length` and
  `max_apdu_header_at_or_below` fail with `Error::Range` instead of
  `Error::Encoding`; the text reads `value out of range:` in place of
  `encoding error:`, with the same message after it.
