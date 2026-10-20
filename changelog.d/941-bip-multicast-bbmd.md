---
section: Added
---
- A B/IP BBMD can serve a multicast group (Annex J.8.3): with `.multicast(...)`
  in BBMD mode it forwards the group's broadcasts to its BDT peers and
  foreign devices and theirs to the group; its own BDT row must be two-hop,
  and its Network Port lists the group read-only (#941).
