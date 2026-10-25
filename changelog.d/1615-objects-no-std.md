---
section: Added
---
- **Breaking (Rust API):** `bacnet-objects` builds without std (`default-features = false, features = ["alloc"]`) on targets with a global allocator and atomic compare-and-swap; the file backends stay behind the default `std` feature, and saves run on the caller's thread (#1615).
