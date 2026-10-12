---
section: Migration notes
---
- **MS/TP core time (Rust API, #1681):** `bacnet-mstp-core` counts
  nanoseconds. Build `now` from a microsecond tick with
  `Instant::from_micros`, give `QueueStorage` `Instant` timestamps, read
  `MstpCounters::de_assertion` (a `Duration`) for `de_assertion_us`, call
  `checked_duration_since` for `duration_since`, and use `Instant::ZERO`
  for `Instant::default()`. `Monotonic` and `SimClock` are gone: keep your
  own clock and build each `Instant` with `from_nanos` or `from_micros`.
