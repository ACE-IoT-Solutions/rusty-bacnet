---
section: Fixed
---
- **Wire:** A direct BACnet/SC listener keeps an accepted link for 360 seconds
  without traffic instead of 60, and counts what it sends as traffic, covering
  every fixed heartbeat period Annex AB.6.3 allows; raise it with
  `with_idle_timeout` for initiators set above 300 seconds (#2133).
