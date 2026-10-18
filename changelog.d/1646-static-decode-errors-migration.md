---
section: Migration notes
---
- **Decoding errors (Rust API, #1646):** match `Error::Decoding(DecodeError)`
  or, for a formatted message, `Error::DecodingMessage { offset, kind, message }`;
  `Error::decode_fault()` reads either. `ObjectIdentifier::new`,
  `DeviceInstanceRange` and an over-long NPDU address fail with `Error::Range`.
  The tagged readers take `&'static str` labels, and `decode_ctx_fixed` a `Fixed`.
