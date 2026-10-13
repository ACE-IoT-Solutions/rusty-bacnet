---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter` runs on the `bacnet-router-core`
  engine. `router_table` re-exports the core's table, with getters on
  `Route`, and `table()` returns a sync `RouterTableGuard` on the live
  table, on `now()`'s clock. A broadcast's relay now queues before its
  local copy (#1679).
