---
section: Changed
---
- `bacnet-bip-core` no longer links a `memmove` into firmware: its BBMD
  tables copy a B/IP address's IP and port apart, and its node engine
  shifts queued events by swaps, saving 0.8 to 1.4 KB of code (#1667).
