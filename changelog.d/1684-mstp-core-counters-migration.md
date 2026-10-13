---
section: Migration notes
---
- **MS/TP counters and clock (Rust API, #1684):** drain
  `MasterCore::take_counters()` into totals of your own (`counters()` looks
  without taking); a `QueueStorage` implements `counters` and
  `take_counters` for `stats`. `MstpDiagnosticsSnapshot::mac` is the
  transport's own `u64` totals. `CoreError::ClockRegression` is gone: an
  earlier `now` is taken as the latest one, and `enqueue_npdu` queues it
  instead of refusing it as `Expired`.
