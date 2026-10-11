---
section: Added
---
- New `bacnet-bip-codec` crate: an allocation-free `no_std` codec for every
  BACnet/IP BVLL message of Annex J, with typed B/IP addresses, borrowed
  BDT/FDT tables and checked encoding into caller storage. The B/IP
  transport now uses it for all BVLL parsing and encoding (#1604).
