---
section: Changed
---
- **Breaking (Rust API):** `AnyTransport` drops its serial-port type parameter: the MS/TP variant boxes its port. `BACnetClient` and `BACnetServer` default to `AnyTransport`, and `build_any()` takes no type parameter (#2039).
