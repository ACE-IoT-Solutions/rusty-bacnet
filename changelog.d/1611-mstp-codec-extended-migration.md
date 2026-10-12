---
section: Migration notes
---
- **MS/TP codec (Rust API, #1611):** match the new `FrameError` variants
  `InvalidCobsLength` and `ExtendedFrame` and the `FrameType` variants for
  Types 32 and 33. `Truncated` and `DataCrcMismatch` name the header's
  Length `length`. Decode extended frames with `decode_frame_in_place` or a
  `StreamDecoder` over `MAX_EXTENDED_FRAME_LENGTH` octets of storage.
