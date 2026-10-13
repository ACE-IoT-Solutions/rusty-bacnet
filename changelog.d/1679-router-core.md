---
section: Added
---
- New `bacnet-router-core` crate: a `no_std` BACnet router (Clause 6) with
  no allocator, a sans-IO engine on injected time and caller storage, with
  an NPDU view and writer, network-layer messages, a routing table, route
  discovery, and B/IP and MS/TP ports over the other cores (`bip`, `mstp`)
  (#1679).
