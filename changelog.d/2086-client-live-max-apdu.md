---
section: Fixed
---
- The client's advertised max-APDU and its request segment size rise again
  when the transport's limit grows after start, such as a router port or an
  SC hub failover publishing a larger one, still capped by the configured
  maximum (#2086).
