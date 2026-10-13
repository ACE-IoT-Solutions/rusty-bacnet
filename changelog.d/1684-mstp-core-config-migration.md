---
section: Migration notes
---
- **MS/TP core config (Rust API, #1684):** `MstpCoreConfig` is
  `#[non_exhaustive]`: start from `MstpCoreConfig::STANDARD` (was
  `standard()`) or `for_baud_rate`, both usable in a `const`, and set the
  fields to change. `n_poll`, `n_retry_token`, `n_min_octets`, `t_no_token`
  and `t_slot` are the constants `N_POLL` and so on, and their
  `ConfigError` variants are gone. Build a `QueueStorage`'s items with
  `DequeuedNpdu::new` and `QueueItemInfo::new`.
