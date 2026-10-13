---
section: Added
---
- New `bacnet-time` crate: the monotonic `Instant` and `Duration`, in
  nanoseconds, that `bacnet-mstp-core`, `bacnet-bip-core` and
  `bacnet-router-core` share, so firmware feeds them one clock. `no_std`, no allocator or dependencies,
  saturating arithmetic and no 64-bit division (#1681).
