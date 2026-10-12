---
section: Changed
---
- B/IP logs an undecodable BVLL header or a malformed BVLC response at debug
  instead of warn, and warns about a full NPDU channel at most once every
  10 seconds with the number dropped; `receive_counters()` counts each
  (#1624).
