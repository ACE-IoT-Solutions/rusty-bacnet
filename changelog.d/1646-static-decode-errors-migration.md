---
section: Migration notes
---
- **Decoding errors (Rust API, #1646, #1776):** match
  `Error::Decoding(DecodeError)`, not the `{ offset, kind, message }`
  variant; `Error::decode_fault()` reads its offset and kind. `decoding`,
  `decoding_kind`, `invalid_tag`, `missing`, `trailing`, `out_of_range`,
  `overflow` and `tagged::misplaced_tag` take a `&'static str`; for numbers
  or context, build a `DecodeError`. `decode_ctx_fixed` takes a `Fixed`, and
  the WPM cursor error holds `error: DecodeError`, not `message`.
