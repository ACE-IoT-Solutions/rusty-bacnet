---
section: Migration notes
---
- **Router groups (Rust API, #1721):** use glue port configurations or
  `PortConfig::new(network, mac, limits, groups)` instead of struct literals.
  Move `Host::group` implementations to `GroupClassifier`; pass that type as
  the last parameter of `Storage` and `Router`. B/IP and MS/TP group helper
  functions and methods are removed; inspect `config.groups.classify(mac)`.
