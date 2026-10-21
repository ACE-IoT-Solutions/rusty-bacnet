---
section: Changed
---
- **Breaking (Rust API):** Codec errors carry static labels, formatted
  only on display: `Error::Decoding` holds a `DecodeError`, encoders refuse
  with `Error::Encode` or `Error::Range`, and `Error::Segmentation` holds a
  `SegmentationError`, so firmware links no formatting (#1646, #1776,
  #1777, #1802).
