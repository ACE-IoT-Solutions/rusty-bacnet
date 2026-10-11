---
section: Changed
---
- **Breaking (Rust API):** `bacnet-types`, `bacnet-encoding` and
  `bacnet-services` build as `no_std` with an allocator on bare-metal targets,
  through a new `alloc` feature; `std` stays the default. Based on work by
  Andrew Rodgers (ACE IoT Solutions) (#1604).
