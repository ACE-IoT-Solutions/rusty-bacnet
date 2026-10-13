---
section: Added
---
- New `bacnet-bip-core` crate: B/IP (Annex J) as `no_std` decisions on
  caller-supplied time with no allocator: a node engine for normal,
  foreign-device and BBMD modes, registration included, and a BBMD's tables
  and broadcast forwarding in caller-sized arrays. The B/IP transport runs
  on it (#1623).
