---
section: Migration notes
---
- **B/IP foreign devices (Rust API, #1622):** In foreign-device mode,
  `register_foreign_device_bvlc`, `delete_fdt_entry` and `write_bdt` to the
  transport's own BBMD fail, since the transport manages that registration.
  Send them from a transport in normal mode, as the CLI does. A foreign
  device's BBMD must be a unicast address with a nonzero port, other than the
  transport's own.
