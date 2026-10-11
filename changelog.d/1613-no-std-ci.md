---
section: Added
---
- Test-only: CI runs the `no_std` crates' tests and doctests with `alloc` and
  without `std`, checks those builds at the MSRV, and lints `bacnet-encoding`,
  `bacnet-services` and the codecs' `bytes` feature on ARMv6-M too (#1613).
