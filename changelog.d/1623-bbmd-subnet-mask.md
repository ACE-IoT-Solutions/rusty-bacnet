---
section: Changed
---
- A B/IP BBMD asks the host for its own subnet's mask at start. Without one
  (a NAT global address, or an OS with no lookup) a forward from a peer on
  its own subnet is still rebroadcast (#1623).
