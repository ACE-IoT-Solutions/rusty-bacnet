---
section: Migration notes
---
- **BbmdState (Rust API, #1623, #1669):** `bdt()` and `fdt()` return
  `Vec`s. Replace `encode_bdt(&mut buf)` with
  `bbmd::encode_bdt_entries(&state.bdt(), &mut buf)`, and `encode_fdt` by
  appending each `fdt_wire_entries()` row's `to_octets()`. In place of
  `forwarding_targets`, `fdt_forwarding_targets`, their `_at` forms and
  `is_registered_foreign_device`, build `bacnet_bip_core::bbmd` tables from
  the rows and ask those. `forwarded_npdu_needs_local_broadcast` goes by
  this BBMD's own BDT row.
