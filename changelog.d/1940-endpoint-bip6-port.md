---
section: Added
---
- **Breaking (Rust API):** The shared endpoint registers a B/IPv6 Network Port: `DeviceIdentity::with_bip6_port` declares it, and an `EndpointSession` composed over a `Bip6Transport` checks, publishes and serves it in NORMAL or FOREIGN mode. `NetworkPortEntry` gains an `ipv6` field (#1940).
