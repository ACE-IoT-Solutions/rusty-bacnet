---
section: Security
---
- **Breaking (Rust API):** A hardened router's control authorizer now sees the
  sending direct BACnet/SC peer's certificate principal, so route changes can be
  limited to configured routers, and SC nodes rate-limit network-layer messages
  per source VMAC (#518).
