---
section: Added
---
- New `bacnet-router-core` crate: a `no_std` BACnet router (Clause 6) with
  no allocator, a sans-IO engine on injected time and caller storage with
  NPDU views and writers, network-layer messages, a routing table and route
  discovery; its `bip` and `mstp` features add ports over the other cores
  (#1679).
