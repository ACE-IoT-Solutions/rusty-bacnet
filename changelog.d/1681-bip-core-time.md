---
section: Changed
---
- **Breaking (Rust API):** `bacnet-bip-core`'s `Instant` and `Duration` are
  `bacnet-time`'s, re-exported at the same paths, in nanoseconds and
  saturating as before; `Duration::half` and `saturating_double` are gone
  (#1681).
