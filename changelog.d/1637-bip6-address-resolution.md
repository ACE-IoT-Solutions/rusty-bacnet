---
section: Added
---
- **Wire:** A B/IPv6 unicast to an unknown VMAC sends Address-Resolution
  (multicast, or to the BBMD as a foreign device) and waits up to 3 s, or queues
  on a router port; `Bip6Transport::resolve_virtual_address` finds a VMAC from
  an IPv6 address (#1637).
