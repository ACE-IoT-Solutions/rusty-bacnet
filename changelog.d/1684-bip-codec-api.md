---
section: Changed
---
- **Breaking (Rust API):** `bacnet-bip-codec`'s `BvllError` and
  `Bvll6Error` are `#[non_exhaustive]`, and its value-returning functions
  are `#[must_use]` (#1684).
