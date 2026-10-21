---
section: Changed
---
- A B/IP ReadProperty round trip in one process costs 13% fewer
  instructions (47,610 to 41,348) and half the bytes allocated: the server's
  request future fits the allocator's fast path, and no task sits between
  transport and application (#1713).
