---
section: Changed
---
- **Wire:** A B/IP foreign device retries a lost registration sooner,
  backing off from the response wait up to the renewal interval, renews into
  the lease a success proved, and registers again early when its BBMD answers
  a broadcast with a Distribute-Broadcast-To-Network NAK (#1622).
