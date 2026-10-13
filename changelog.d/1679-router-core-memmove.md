---
section: Changed
---
- `bacnet-router-core` no longer links a `memmove` into firmware: its
  routing table, route searches, hold-down records and output queue shift
  and compact by swaps, saving 0.8 to 1.4 KB of code (#1679).
