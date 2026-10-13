---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter` runs on the `bacnet-router-core`
  engine. `router_table` re-exports the core's table types, and
  `BACnetRouter::table()` returns a sync guard over the live table, with
  times on the router's clock, `BACnetRouter::now()` (#1679).
