---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` counts time in nanoseconds
  with `bacnet-time`'s `Instant` and `Duration`, re-exported at its root,
  and every timer keeps its value. Its queue storage takes `Instant`s, and
  `Monotonic` and `SimClock` are gone (#1681).
