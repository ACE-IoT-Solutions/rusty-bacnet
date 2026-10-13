---
section: Migration notes
---
- **BBMD forwarding (Rust API, #1669):** In place of the removed `BbmdState`
  helpers, read a live BBMD's rows with `BbmdState::bdt()` and `fdt()`, build
  `bacnet_bip_core::bbmd` tables from them, and ask those where a broadcast
  goes (`Tables::broadcast_targets`, `Tables::foreign_device_targets`) or
  whether a device is registered (`Fdt::is_registered_at`).
