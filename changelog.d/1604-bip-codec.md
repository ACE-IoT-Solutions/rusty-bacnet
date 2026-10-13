---
section: Added
---
- New `bacnet-bip-codec` crate: an allocation-free `no_std` codec for every
  B/IP (Annex J) and B/IPv6 (Annex U) BVLL message, with typed addresses and
  VMACs, borrowed BDT/FDT views and checked encoding into caller storage.
  The B/IP and B/IPv6 transports parse and encode BVLL with it (#1604,
  #1621).
