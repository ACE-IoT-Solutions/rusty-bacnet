---
section: Changed
---
- **Wire:** A B/IP foreign device retries a lost registration sooner,
  backing off from the response wait up to the renewal interval, renews into
  the lease a success proved, registers again early when its BBMD NAKs a
  Distribute-Broadcast-To-Network, and logs each lapse once (#1622).
