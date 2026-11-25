---
section: Added
---
- Add a rootless host-network probe for B/IP endpoints sharing a port across containers. It checks broadcast discovery, source addresses, readiness, bind refusal and recovery with the other endpoint still serving reads. Each container verifies its mounted inputs before opening a BACnet socket.
