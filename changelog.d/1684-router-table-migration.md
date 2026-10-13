---
section: Migration notes
---
- **Router routes (Rust API, #1684):** read a `Route` through `port()`,
  `is_direct()`, `next_hop()`, `reachability()`, `busy_until()` and its
  other getters; ports are `PortId`s. `lookup_mut` is gone: change a route
  with `add_learned` (port, next hop, `last_seen`), `add_direct`,
  `mark_busy`, `mark_available` or `mark_unreachable` (state) and `touch`
  (`last_used`). A direct route turns learned by `remove`, then
  `add_learned`. `new_hardened()` is `new()` and
  `set_convergence(Convergence::Hardened)`.
