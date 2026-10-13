---
section: Fixed
---
- **Wire:** `BACnetRouter` answers an NPDU too long for the port it would
  leave by with reject reason 4 to its source, counted in `egress_too_long`,
  where it used to drop it silently at that port's send. A global broadcast
  skips such a port, counted, with no reject (#1665).
