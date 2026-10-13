---
section: Changed
---
- `BACnetRouter`'s routing table, route search and aging run on Tokio time
  from the router's start instead of the wall clock, so paused-time tests
  drive them, and the aging sweep runs at the engine's own deadlines
  (#1679).
