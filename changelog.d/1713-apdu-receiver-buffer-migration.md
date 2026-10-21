---
section: Migration notes
---
- **NetworkLayer buffering (#1713):** the raw receiver has no 256-APDU queue
  of its own, so the transport's queue is the only buffer: 256 NPDUs on most
  links, 64 on MS/TP (before, 64 + 256). Read overload drops from the
  transport's counters. A router's local delivery now carries
  `router_response` only on a Confirmed-Request.
