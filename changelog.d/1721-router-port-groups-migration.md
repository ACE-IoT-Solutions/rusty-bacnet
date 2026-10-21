---
section: Migration notes
---
- **Router groups (Rust API, #1721):** use glue port configurations or
  `PortConfig::new(network, mac, limits, groups)` instead of struct literals.
  Move `Host::group` implementations to a `GroupClassifier` and pass it as
  the `G` parameter of `Storage` and `Router`, after `N`. The B/IP and MS/TP
  group helper functions and methods are removed; call
  `config.groups.classify(mac)`.
