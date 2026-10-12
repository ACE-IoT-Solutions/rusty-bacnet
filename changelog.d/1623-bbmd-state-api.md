---
section: Changed
---
- **Breaking (Rust API):** `bbmd::BbmdState::bdt()` and `fdt()` return copies
  of the rows, and `encode_bdt()` and `encode_fdt()` are removed, now that the
  tables live in `bacnet-bip-core` (#1623).
