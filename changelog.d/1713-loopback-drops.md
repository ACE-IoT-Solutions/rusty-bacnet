---
section: Changed
---
- `LoopbackTransport` drops a frame that finds the peer's queue full, as a
  real data link does, and counts it in the peer's `receive_drops()`, instead
  of making the sender wait (#1713).
