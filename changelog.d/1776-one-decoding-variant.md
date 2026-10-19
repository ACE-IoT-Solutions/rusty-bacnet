---
section: Changed
---
- **Breaking (Rust API):** Every decoder now reports `Error::Decoding` with a
  static label, so no refusal formats text when it is made and
  `Error::DecodingMessage` is gone (#1776).
