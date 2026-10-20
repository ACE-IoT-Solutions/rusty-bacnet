---
section: Changed
---
- **Breaking (Rust API):** `bacnet_transport::port::BipPort` is
  `#[non_exhaustive]` with `BipPort::new`, and gains a `multicast` field,
  the group view a Network Port serves (#941).
