---
section: Fixed
---
- **Wire:** A direct BACnet/SC listener keeps an idle accepted link for 360
  seconds by default instead of 60, so an initiator using any heartbeat period
  Annex AB.6.3 allows stays connected; `DirectListener::start` now rejects a
  zero idle timeout (#2133).
