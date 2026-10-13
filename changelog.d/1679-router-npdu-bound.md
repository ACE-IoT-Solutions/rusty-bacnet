---
section: Changed
---
- **Wire:** `BACnetRouter` no longer routes an NPDU whose rewrite is longer
  than 65,556 octets, the largest BACnet/SC NPDU plus the source address a
  router adds; it is dropped, logged and counted in the new
  `BACnetRouter::counters().write_failures` (#1679).
