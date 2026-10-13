---
section: Migration notes
---
- **MS/TP frame types (Rust API, #1684):** `FrameType` wraps the type
  octet, and each variant is now a constant named in upper snake case:
  `Token` is `TOKEN`, `PollForMaster` is `POLL_FOR_MASTER`,
  `BACnetDataExpectingReply` is `BACNET_DATA_EXPECTING_REPLY`, and so on.
  A `match` on it needs a `_` arm. `FrameType::Unknown(raw)` becomes
  `FrameType::from_raw(raw)`, equal to any frame type with that octet;
  match one with a guard on `to_raw()`.
