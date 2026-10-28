---
section: Migration notes
---
- **Registered B/IPv6 port (Rust API, #2057):** build the `Bip6Transport`
  with `Some(instance)` of the Device the server or endpoint serves; a
  different instance or `None` (a random VMAC) now fails at start, as does
  an endpoint with no `DeviceIdentity` and no Device in its database
  ("requires a served Device"). `network_port_registration::bip6_address`
  takes the served instance as a second argument.
