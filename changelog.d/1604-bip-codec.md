---
section: Added
---
- New `bacnet-bip-codec` crate: an allocation-free `no_std` codec for every
  B/IP (Annex J) and B/IPv6 (Annex U) BVLL message, with typed addresses,
  VMACs and BDT/FDT views; `alloc`, `bytes` and `heapless` add encode sinks.
  The B/IP and B/IPv6 transports parse and encode BVLL with it (#1604,
  #1621).
