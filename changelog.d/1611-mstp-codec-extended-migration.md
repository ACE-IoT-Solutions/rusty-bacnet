---
section: Migration notes
---
- **MS/TP codec (Rust API, #1611):** `decode_frame` refuses an extended
  frame with `FrameError::ExtendedFrame`; decode one with
  `decode_frame_in_place`, or a `StreamDecoder` over
  `MAX_EXTENDED_FRAME_LENGTH` octets of storage.
