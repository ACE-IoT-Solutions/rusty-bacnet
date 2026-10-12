---
section: Migration notes
---
- **Portable core time (Rust API, #1681):** `bacnet-mstp-core` counts
  nanoseconds. Build `now` from a microsecond tick with
  `Instant::from_micros` and lengths with `Duration::from_micros`, give
  `QueueStorage` `Instant` timestamps, read `MstpCounters::de_assertion`, a
  `Duration`, for `de_assertion_us`, and call `checked_duration_since` for
  `duration_since`. `bacnet-bip-core` imports stay; use `saturating_mul(2)`
  for `saturating_double`.
