---
section: Changed
---
- **Wire:** `BACnetRouter` no longer routes an NPDU whose rewrite is longer
  than 65,556 octets: the largest BVLC message plus the source address a
  router adds, which any BACnet/SC NPDU fits. It is dropped, logged and
  counted in `BACnetRouter::counters().write_failures` (#1679).
