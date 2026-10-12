---
section: Changed
---
- **Breaking (Rust API):** `bbmd::BbmdState::bdt()` and `fdt()` return copies
  of the rows, `encode_bdt()` and `encode_fdt()` are removed, and
  `forwarded_npdu_needs_local_broadcast` follows the corrected J.4.5 rule, now
  that the tables live in `bacnet-bip-core` (#1623).
