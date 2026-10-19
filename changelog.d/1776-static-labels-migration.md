---
section: Migration notes
---
- **Decoding constructors (Rust API, #1776):** `Error::decoding`,
  `decoding_kind`, `invalid_tag`, `missing`, `trailing`, `out_of_range`,
  `overflow` and `tagged::misplaced_tag` take a `&'static str`; for numbers
  or a context, build a `DecodeError`. New: `Error::to_decode_error`,
  `tagged::misplaced`, `DecodeError::at_item` and
  `WritePropertyMultipleDecodeStage::name`.
