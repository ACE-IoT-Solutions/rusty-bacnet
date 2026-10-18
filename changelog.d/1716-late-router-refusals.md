---
section: Fixed
---
- Routers return a length rejection to an eligible sender when a queued
  transmission exceeds the transport's final limit, including BACnet/SC
  frame attributes and direct-peer limits. Rejected replies never trigger
  another reply (#1716).
