---
section: Migration notes
---
- **Managed COV targets (Rust API, #1790):** `manage_cov_subscription` and
  `manage_cov_property_subscription` take a `CovTarget` in place of the
  destination MAC: `CovTarget::local(mac)`, `CovTarget::routed(router_mac,
  network, address)` or `CovTarget::of_device(&device)`. Cancel with
  `unsubscribe_cov_target` or `unsubscribe_cov_property_target` at the same
  target.
