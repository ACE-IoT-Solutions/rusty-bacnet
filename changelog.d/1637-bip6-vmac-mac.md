---
section: Changed
---
- **Breaking (Rust API, wire):** A B/IPv6 transport's MAC is now the node's
  3-octet VMAC (Annex U): `local_mac()`, received source MACs, `send_unicast`
  and routed SADR/DADR use it (#1637).
