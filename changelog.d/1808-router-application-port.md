---
section: Added
---
- `BACnetRouter::application_port` makes the router's own application a transport, so a `BACnetClient` built on it reads and writes devices through the routing table, with invoke IDs, timeouts and segmentation (#1808).
