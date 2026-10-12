---
section: Migration notes
---
- **B/IP core time (Rust API, #1681):** `bacnet_bip_core::{Duration,
  Instant}` keep their paths. Use `Instant::ZERO` for `Instant::default()`,
  `saturating_mul(2)` for `saturating_double`, and for `half` build the
  half directly (half a time-to-live is `Duration::from_millis(ttl * 500)`).
  Convert a `core::time::Duration` with `Duration::try_from`, which refuses
  one past about 584 years, or the `const` `Duration::saturating_from_core`.
