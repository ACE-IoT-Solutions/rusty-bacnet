---
section: Changed
---
- `bacnet_types::bip_port` builds with the `alloc` feature on targets with atomic compare-and-swap, not only with `std`, so a no_std device can lend its B/IP state to the Network Port object (#1615).
