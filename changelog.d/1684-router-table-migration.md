---
section: Migration notes
---
- **Router table (Rust API, #1684):** read a `Route` through `port()`,
  `is_direct()`, `next_hop()`, `reachability()`, `busy_until()` and its
  other getters. `Table::lookup_mut` is gone: change a route with
  `add_learned` (port, next hop, `last_seen`), `add_direct`, `mark_busy`,
  `mark_available` or `mark_unreachable` (state) and `touch` (`last_used`);
  the flap count is the table's.
