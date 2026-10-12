---
section: Changed
---
- B/IPv6 logs an undecodable frame at debug; its full NPDU channel, VMAC
  collision warning and BVLC-Result NAK error, and a B/IP BBMD's full fan-out
  queue and failed forwarding sends, log at most once per 10 s with a count;
  a stopped fan-out worker logs at debug (#1643).
