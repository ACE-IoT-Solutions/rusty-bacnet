---
section: Changed
---
- **Breaking (Rust API):** `bacnet_types::error::Error` is
  `#[non_exhaustive]`: its `std` feature adds a variant, and any crate in a
  build can turn that on (#1604).
