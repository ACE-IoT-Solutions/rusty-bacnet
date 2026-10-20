---
section: Security
---
- **Breaking (Rust API):** One `ServerPolicy` now makes the server's
  authorization decisions, with documented defaults, and each context carries
  the verified direct BACnet/SC certificate principal (`ScPrincipal`); hub-relayed
  and non-SC requests carry none (#524).
