---
section: Fixed
---
- Routers return a length rejection to an eligible sender when BACnet/SC
  reports a definite length refusal for a queued transmission, including
  frame attributes and direct-peer limits. Rejected replies never trigger
  another reply (#1716).
