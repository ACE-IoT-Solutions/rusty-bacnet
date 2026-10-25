---
section: Changed
---
- **Breaking (Rust API):** `Bip6Transport::start` refuses a foreign-device registration whose BBMD isn't an IPv6 unicast host (link-local, IPv4-mapped and the like); `Bip6ForeignDeviceConfig::check` runs the same test (#1941).
