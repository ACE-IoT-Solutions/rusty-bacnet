---
section: Migration notes
---
- **MS/TP transport (Rust API, #1611):** add `extended_frames: false`, or
  `..MstpConfig::default()`, to an `MstpConfig` literal that lists every
  field, and have a `TransportPort` wrapper forward `prepare_for_routing`.
  A router's MS/TP ports now declare an APDU limit of 1476.
