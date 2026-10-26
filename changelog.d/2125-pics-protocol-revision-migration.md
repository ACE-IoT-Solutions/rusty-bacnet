---
section: Migration notes
---
- **PICS (Rust API, #2125):** Remove `protocol_version` and
  `protocol_revision` from your `PicsConfig`; `generate_pics` reads both from
  the Device object. Code that read them from the Device to fill those fields
  can drop that step. `bacnet_types::PROTOCOL_VERSION` and
  `bacnet_types::PROTOCOL_REVISION` hold the values the bundled Device reports.
