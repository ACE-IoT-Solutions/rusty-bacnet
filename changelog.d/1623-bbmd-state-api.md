---
section: Changed
---
- **Breaking (Rust API):** `bbmd::BbmdState` keeps its tables in
  `bacnet-bip-core`: `bdt()` and `fdt()` return copies of the rows,
  `encode_bdt()`, `encode_fdt()` and the forwarding helpers are gone, and
  `forwarded_npdu_needs_local_broadcast` follows the corrected J.4.5 rule
  (#1623, #1669).
