---
section: Migration notes
---
- **Router table (Rust API, #1679):** drop the `.lock().await` after
  `router.table()`, and don't hold the guard across an await or call
  `table()` again while it lives. `RouteEntry`, `ReachabilityStatus`,
  `ConvergenceMode` and `RoutingClaimSnapshot` are `Route`, `Reachability`,
  `Convergence` and `ClaimCounters` (`claim_counters()`). Calls that take a
  time take `router.now()`. `add_learned` and `add_direct` return `bool`,
  refusing a new network once the table holds 256 routes (`ROUTE_CAP`).
