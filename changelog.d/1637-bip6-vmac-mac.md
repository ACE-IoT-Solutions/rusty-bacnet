---
section: Changed
---
- **Breaking (Rust API, wire):** The B/IPv6 transport's network-layer MAC is now
  the node's 3-octet VMAC, as Annex U requires: `local_mac()`, received source
  MACs and `send_unicast` use it, and routed SADR/DADR for B/IPv6 nodes are 3
  octets (#1637).
