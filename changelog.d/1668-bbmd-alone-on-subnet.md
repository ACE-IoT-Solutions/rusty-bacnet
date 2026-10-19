---
section: Added
---
- A BBMD alone on its subnet, as behind a NAT router, can skip the local
  rebroadcast of forwarded and distributed broadcasts:
  `BbmdOptions::alone_on_subnet`, `LocalSubnet::alone` in `bacnet-bip-core`,
  and **Python API:** `Bbmd(alone_on_subnet=True)`. Off by default (#1668).
