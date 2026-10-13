---
section: Added
---
- New `bacnet-time` crate: the monotonic `Instant` and `Duration`, in
  nanoseconds, that the MS/TP, B/IP and router cores share, for one firmware
  clock. `no_std`, with no features, allocator or dependencies, saturating
  arithmetic and no 64-bit division. Like each new crate, it needs Rust 1.93
  (#1681).
