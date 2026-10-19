---
section: Added
---
- **Python API:** `BACnetClient` collects Who-Has answers with
  `who_has_replies()` and routers with `discover_routers()`, and follows
  received I-Haves and network reports through `i_have_events()` and
  `network_reports()`, async iterators that never hold up the client (#1850).
