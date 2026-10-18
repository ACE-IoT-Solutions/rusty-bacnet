---
section: Migration notes
---
- **Router core (Rust API, #1767):** `router::Counters` fields are `u32`
  counts since the last `Router::take_counters()`; add them to your own
  totals. `Options::route_cap` is an `Option<u16>` (`None` fills the table),
  and `Options::default()` is `STANDARD`. `bacnet_network::router::Counters`
  is now its own `u64` type, not a re-export.
