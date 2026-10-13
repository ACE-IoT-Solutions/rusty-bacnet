---
section: Fixed
---
- **Wire:** `BACnetRouter` answers an NPDU too long for the port it would
  leave by with reject reason 4 to its source, counted in `egress_too_long`.
  That port's send used to fail it, with a warning, and the source heard
  nothing. A global broadcast skips such a port, with no reject (#1665).
