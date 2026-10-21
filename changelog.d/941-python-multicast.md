---
section: Added
---
- **Python API:** `BACnetClient`, `BACnetServer` and `BipEndpoint` take
  `multicast=Multicast(group, ttl=1, port=None)` to join a B/IP multicast
  group, and with `bbmd=Bbmd(...)` serve it as its BBMD (#941).
