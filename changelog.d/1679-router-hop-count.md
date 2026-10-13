---
section: Fixed
---
- **Wire:** `BACnetRouter` no longer passes on, with a hop count of 0, an
  NPDU for another router or a global broadcast that arrives with 1; it
  counts it in `hop_count_drops`, still taking the broadcast itself. Delivery
  onto an attached network is unchanged (#1679).
