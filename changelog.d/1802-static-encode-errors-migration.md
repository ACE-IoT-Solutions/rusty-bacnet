---
section: Migration notes
---
- **Encoding and range errors (Rust API, #1646, #1776, #1777, #1802):** match
  `Error::Encode(EncodeError)`, not `Error::Encoding`, for codec encoder
  refusals, and `Error::Range`, not `Error::Encoding` or `Error::OutOfRange`,
  for range checks: the APDU and NPDU encoders and validators,
  `ObjectIdentifier::new`, `DeviceInstanceRange`, an over-long NPDU address
  and oversized lighting, color and elevator command members.
  `Error::Segmentation` holds a `SegmentationError` enum.
