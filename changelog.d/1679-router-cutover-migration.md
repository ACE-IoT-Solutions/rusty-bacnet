---
section: Migration notes
---
- **Router table (Rust API, #1679):** Drop the `.lock().await` after
  `router.table()` and don't hold the guard across an await. `RouteEntry`,
  `ReachabilityStatus`, `ConvergenceMode` and `RoutingClaimSnapshot` are
  `Route`, `Reachability`, `Convergence` and `ClaimCounters`; a route's
  `port_index`, `directly_connected` and `next_hop_mac` are the getters
  `port()`, `is_direct()` and `next_hop()`. Calls that take a time take `router.now()`,
  and `claim_snapshot()` is `claim_counters()`.
