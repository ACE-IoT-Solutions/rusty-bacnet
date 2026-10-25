---
section: Migration notes
---
- **Managed COV targets (Rust API, #1790):** `manage_cov_subscription` and
  `manage_cov_property_subscription` take a `CovTarget` in place of the
  destination MAC: pass `CovTarget::local(mac)` for a device on this network,
  `CovTarget::routed(router_mac, network, address)` for one behind a router,
  or `CovTarget::of_device(&device)` for a discovered device.
