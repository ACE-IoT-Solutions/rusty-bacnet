---
section: Changed
---
- **Breaking (Rust API):** The encoders and validators in bacnet-types,
  bacnet-encoding and bacnet-services refuse with static `Error::Encode` or
  `Error::Range` errors, and segmentation refusals are a typed
  `SegmentationError`, so the codec crates format no error text (#1802).
