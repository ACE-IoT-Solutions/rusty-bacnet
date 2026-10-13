---
section: Migration notes
---
- **MS/TP transport (Rust API, #1655):** add `usage_timeout` and
  `reply_timeout`, or `..MstpConfig::default()`, to an `MstpConfig` literal
  that lists every field. To keep the old waits, set them to 20 ms and
  255 ms; `validate()` refuses values outside 20 to 35 ms and 255 to 300 ms.
