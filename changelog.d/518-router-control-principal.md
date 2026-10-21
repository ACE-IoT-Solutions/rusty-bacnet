---
section: Security
---
- **Breaking (Rust API):** A hardened router's control authorizer sees the
  sending direct BACnet/SC peer's certificate principal for unicast controls (SC
  broadcasts arrive hub-relayed, with none), and SC nodes rate-limit
  network-layer messages per certificate or VMAC (#518).
