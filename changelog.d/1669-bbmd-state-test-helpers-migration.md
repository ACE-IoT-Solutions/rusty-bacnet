---
section: Migration notes
---
- **BBMD forwarding (Rust API, #1669):** To ask where a broadcast would go,
  or whether a foreign device is registered, use the decisions in
  `bacnet_bip_core::bbmd` (`Tables::broadcast_targets`,
  `Tables::foreign_device_targets`, `Fdt::is_registered_at`) in place of the
  removed `BbmdState` helpers.
