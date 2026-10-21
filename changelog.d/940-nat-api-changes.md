---
section: Changed
---
- **Breaking (Rust API):** `BbmdOptions` gains `nat` and `alone_on_subnet`,
  `DataLinkSupport` is `#[non_exhaustive]` with `BipV4NatTraversal`,
  `bip_core::bbmd::Tables` gains a private field, and
  `BbmdState::local_address` returns the socket's `SocketAddrV4` even behind
  NAT (#940, #1668).
