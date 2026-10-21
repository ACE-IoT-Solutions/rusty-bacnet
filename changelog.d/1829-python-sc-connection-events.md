---
section: Added
---
- **Python API:** An SC `BACnetClient` takes `sc_failover_hub` and
  `sc_connect_timeout_ms`, and reports its hub link through
  `connection_events()`, an async iterator whose first call starts with the
  first `connected`, and `connection_status()` (#1829, #1839, #1908).
