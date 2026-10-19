---
section: Changed
---
- **Breaking (Rust API):** The APDU and NPDU encoders refuse an out-of-range
  max-segments, max-APDU, window size, network or address length with a
  static `Error::Range`, so the apdu firmware image links no `core::fmt`
  (#1777).
