---
section: Changed
---
- **Breaking (Rust API):** The B/IP client, server and endpoint builders take
  one `BipMode` (normal, foreign device or BBMD) and report the foreign
  registration; Python's B/IP owners take `bbmd=` and `foreign_device=`, and
  the CLI `--foreign-bbmd`, `--ttl` and `--renewal` (#938).
