---
section: Changed
---
- **Breaking (Rust API):** `BipEndpointBuilder` takes one `BipMode` (normal,
  foreign device or BBMD) through `.mode(..)` in place of
  `register_as_foreign_device`, `enable_bbmd` and the BBMD setters (#938).
