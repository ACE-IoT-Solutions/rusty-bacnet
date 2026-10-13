---
section: Migration notes
---
- **MS/TP frame types (Rust API, #1684):** `FrameType` wraps the type
  octet, and its variants are constants: `Token` is `TOKEN`,
  `BACnetDataExpectingReply` is `BACNET_DATA_EXPECTING_REPLY`, and so on.
  A `match` on it needs a `_` arm. `FrameType::Unknown(raw)` becomes
  `FrameType::from_raw(raw)`, which equals any frame type with that octet;
  match one with a guard on `to_raw()`.
