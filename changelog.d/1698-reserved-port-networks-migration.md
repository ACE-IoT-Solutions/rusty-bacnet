---
section: Migration notes
---
- **Router port numbers (Rust API, #1698):** give every `RouterPort` and
  core `PortConfig` a network number from 1 to 65534. For 0 or 0xFFFF,
  `BACnetRouter::start` now fails before starting any transport, and
  `Router::new` returns the new `ConfigError::ReservedNetwork`.
