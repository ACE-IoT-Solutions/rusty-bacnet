---
section: Added
---
- `bacnet-bip-codec` gains an `ipv6` module: an allocation-free `no_std`
  codec for every BACnet/IPv6 BVLL message of Annex U, with typed B/IPv6
  addresses and VMACs and checked encoding. The B/IPv6 transport now uses it
  for all BVLL parsing and encoding (#1621).
