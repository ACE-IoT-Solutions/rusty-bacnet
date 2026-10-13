---
section: Changed
---
- **Breaking (Rust API):** `bacnet-router-core` settles its API before it
  first publishes: no `Fixes` (it always follows Clause 6),
  `#[non_exhaustive]` types, wire-code newtypes, `Route` getters, drained
  port counters, and an early `now` taken as the latest one (#1684).
