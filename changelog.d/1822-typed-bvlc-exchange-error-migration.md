---
section: Migration notes
---
- **BVLC management errors (Rust API, #1822):** `read_bdt`, `read_fdt`,
  `write_bdt`, `delete_fdt_entry` and `register_foreign_device_bvlc` return
  `Result<_, BvlcExchangeError>`. Match `Nak(code)` (or call `nak()`) instead
  of the `"BVLC-Result: …"` encoding-error text, and `NoReply` instead of
  `Error::Timeout`; other errors arrive as `Transport(Error)`. The last three
  return `Ok(())` on success instead of a result code, and a NAK as `Err`.
