---
section: Migration notes
---
- **Router routes (Rust API, #1684):** read a `Route` through `port()`,
  `is_direct()`, `next_hop()` and its other getters; `lookup` returns a
  copy, ports are `PortId`s and MACs `Mac`s. `lookup_mut` is gone: change a
  route with `add_learned`, `add_direct`, `mark_busy`, `mark_available`,
  `mark_unreachable` and `touch`. The `_at` and flap-detection helpers,
  `purge_stale`, `expire_pending_at` and the listings changed too: see
  [moving a router table](docs/rust-api.md#moving-a-router-table-from-012).
