---
section: Migration notes
---
- **B/IPv6 foreign device (Rust API, #1941):** `Bip6Transport::start` now fails with `InvalidInput` when the `Bip6ForeignDeviceConfig` names an unspecified, multicast, link-local, IPv4-mapped or IPv4-compatible BBMD, port 0 or TTL 0. Such a registration never worked reliably; call `Bip6ForeignDeviceConfig::check` first to report it, and give a unicast BBMD address.
