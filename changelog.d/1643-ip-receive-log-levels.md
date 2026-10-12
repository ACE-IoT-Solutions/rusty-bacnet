---
section: Changed
---
- B/IPv6 logs an undecodable frame at debug instead of warn, and B/IPv6's full
  NPDU channel and the B/IP BBMD's full fan-out queue and failed forwarding
  sends warn at most once every 10 seconds with the count since the last
  warning (#1643).
