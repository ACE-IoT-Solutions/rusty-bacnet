---
section: Added
---
- **Python API:** `foreign_device=` works with `transport="ipv6"` on `BACnetClient` and `BACnetServer`: `ForeignDevice` takes a B/IPv6 BBMD as `"ip"` or `"[ip]:port"`, and a registered Network Port reports foreign mode. The Rust B/IPv6 client builder gains `foreign_device(..)` (#1941).
