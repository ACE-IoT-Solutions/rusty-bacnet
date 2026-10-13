---
section: Added
commit: 8042aef046537d61cc4f46136b0d1d5747632cec
---
- **Rust and Python API, CLI:** the B/IP client and server builders take
  `.mode(BipMode::..)` and report the foreign registration; Python's B/IP
  owners take `bbmd=` and `foreign_device=`, and the CLI `--foreign-bbmd`,
  `--ttl` and `--renewal` (#938).
