---
section: Fixed
---
- **Wire:** A router drops a global broadcast or Who-Is-Router whose source
  address is its own, its message come back round a loop, so its application
  no longer receives its own broadcast; counted in `own_source_drops`
  (#2021).
