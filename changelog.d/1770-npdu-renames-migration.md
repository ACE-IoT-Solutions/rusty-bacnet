---
section: Migration notes
---
- **More network-layer renames (Rust API, #1770):** `BACNET_PROTOCOL_VERSION`
  is `bacnet_npdu::VERSION`, `NpduAddressField` is
  `bacnet_npdu::AddressField`, router-core's `Limits::MIN` is
  `router::MIN_PORT_LIMIT`, and `bacnet_mstp_core::{NetworkPriority,
  PriorityError}` are under `bacnet_mstp_core::npdu`. bacnet-encoding
  re-exports the crate as `bacnet_encoding::npdu::view`.
