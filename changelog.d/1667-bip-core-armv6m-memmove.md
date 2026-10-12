---
section: Changed
---
- `bacnet-bip-core`'s BBMD tables copy a B/IP address's IP and port
  apart, so ARMv6-M (Cortex-M0) firmware no longer links a 1.3 KB
  `memmove` for them (#1667).
