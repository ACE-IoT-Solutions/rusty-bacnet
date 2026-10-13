---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter` runs on the `bacnet-router-core`
  engine. `router_table` re-exports the core's table types, and `table()`
  returns a `RouterTableGuard`, a sync guard on the live table, whose times
  are `now()`'s. A broadcast's relay now queues before its local copy
  (#1679).
