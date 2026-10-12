---
section: Added
---
- New `bacnet-time` crate: the monotonic `Instant` and `Duration`, in
  nanoseconds, that `bacnet-mstp-core` and `bacnet-bip-core` share, so
  firmware feeds them one clock. `no_std`, no allocator or dependencies,
  saturating arithmetic and no 64-bit division (#1681).
