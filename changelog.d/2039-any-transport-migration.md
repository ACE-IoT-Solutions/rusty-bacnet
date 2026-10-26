---
section: Migration notes
---
- **AnyTransport (Rust API, #2039):** Write `AnyTransport` for `AnyTransport<S>`, and `build_any()` or `build_any_with_events()` without a turbofish; `BACnetClient` and `BACnetServer` alone now name the `AnyTransport` forms. `NoSerial` is gone. Build the MS/TP variant with `MstpTransport::new(port, config).into()`, or name its type as `MstpTransport<BoxedSerial>`.
