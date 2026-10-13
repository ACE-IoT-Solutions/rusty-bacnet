---
section: Changed
---
- **Wire:** `BACnetRouter` doesn't pass on a global broadcast whose rewrite
  is longer than 65,556 octets, the largest BVLC message plus the source a
  router adds; it is dropped, logged and counted in `write_failures`. Other
  NPDUs meet their port's limit first (#1665, #1679).
