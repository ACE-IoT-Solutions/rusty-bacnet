---
section: Migration notes
---
- **MS/TP frame types (Rust API, #1684):** `FrameType` wraps the type
  octet; each variant is a constant in upper snake case (`Token` is
  `TOKEN`, `BACnetDataExpectingReply` is `BACNET_DATA_EXPECTING_REPLY`).
  Give a `match` a `_` arm. `Unknown(raw)` is `FrameType::from_raw(raw)`;
  match it with a guard on `to_raw()`. `Debug` prints `FrameType::TOKEN`,
  or `FrameType(128)` for an unnamed type, which changes log text.
