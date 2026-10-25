---
section: Migration notes
---
- **Registered B/IPv6 port (Rust API, #2057):** build the `Bip6Transport`
  with `Some(instance)` of the Device the server or endpoint serves; a
  different instance or `None` (a random VMAC) now fails at start.
  `network_port_registration::bip6_address` takes that instance as a second
  argument.
