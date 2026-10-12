---
section: Added
---
- New `bacnet-bip-core` crate: a BBMD's BDT and FDT in caller-sized arrays,
  foreign-device registration and broadcast forwarding (Annex J.4, J.5) as
  `no_std` decisions on caller-supplied time, with no allocator. The B/IP
  transport's BBMD now runs on it (#1623).
