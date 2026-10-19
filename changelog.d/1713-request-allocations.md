---
section: Changed
---
- A B/IP ReadProperty round trip, client and server in one process, costs
  about 11% fewer instructions and 22 heap allocations instead of 29: the
  server runs ReadProperty in a smaller task, encodes each reply once, and
  the client and server hash their transaction tables more cheaply (#1713).
