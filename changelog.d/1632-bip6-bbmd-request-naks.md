---
section: Changed
---
- **Wire:** A B/IPv6 node now answers a Register-Foreign-Device,
  Delete-Foreign-Device-Table-Entry or Distribute-Broadcast-To-Network sent to
  it with that request's NAK, and a foreign device answers a
  Forwarded-Address-Resolution from its BBMD. Both replies are rate-limited
  (#1632).
