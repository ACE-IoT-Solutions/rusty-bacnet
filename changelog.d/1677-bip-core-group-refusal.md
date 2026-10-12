---
section: Changed
---
- **Breaking (Rust API):** `bacnet_bip_core::node::SendRefusal` gains
  `GroupDestination`, which `Link::unicast` returns for a group address of
  the link (#1677).
