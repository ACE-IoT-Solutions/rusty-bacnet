---
section: Changed
---
- **Breaking (Rust API):** `bacnet-router-core` drains its counters like the
  link cores (`take_counters()` of `u32`s, `counters()` to look), re-exports
  `bip_core`/`mstp_core`, and adds `RouterIn` and `Options::STANDARD`;
  `BACnetRouter` keeps its `u64` totals (#1767).
