---
section: Migration notes
---
- **BbmdState (Rust API, #1623):** `bdt()` returns `Vec<BdtEntry>` and `fdt()`
  returns `Vec<FdtEntry>`; borrow them with `&` where a slice was expected.
  Replace `encode_bdt(&mut buf)` with
  `bbmd::encode_bdt_entries(&state.bdt(), &mut buf)`, and `encode_fdt` by
  appending each `fdt_wire_entries()` row's `to_octets()`.
