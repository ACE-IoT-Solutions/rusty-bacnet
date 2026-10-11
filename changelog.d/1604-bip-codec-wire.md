---
section: Changed
---
- **Wire:** the B/IP transport drops a datagram whose BVLC Length field is
  not its length instead of reading the shorter message, and a BBMD NAKs a
  Read-BDT or Read-FDT that carries a payload instead of answering it (#1604).
