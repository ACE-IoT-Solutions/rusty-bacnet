---
section: Added
---
- **Python API:** An SC `BACnetClient` takes `sc_failover_hub` and
  `sc_connect_timeout_ms`, and reports its hub link through
  `connection_events()`, an async iterator that never holds up the
  connector, and `connection_status()` (#1829).
