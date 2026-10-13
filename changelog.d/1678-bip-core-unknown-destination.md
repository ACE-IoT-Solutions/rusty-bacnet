---
section: Changed
---
- **Breaking (Rust API):** a `bacnet-bip-core` host that can't tell where a
  datagram was sent now says so, and the node drops the datagram, counted
  as `Discard::UnknownDestination`, instead of acting on a guess (#1678).
