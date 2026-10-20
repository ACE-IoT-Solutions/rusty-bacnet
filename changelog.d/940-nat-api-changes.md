---
section: Changed
---
- **Breaking (Rust API):** `BbmdOptions` gains `nat`, `DataLinkSupport` gains
  `BipV4NatTraversal`, `bip_core::bbmd::Tables` gains a private field, and
  `BbmdState::local_address` is the socket's address even behind NAT (#940).
