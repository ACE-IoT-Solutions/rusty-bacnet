---
section: Migration notes
---
- **Device identity (#1720):** Use `DeviceIdentity`, `ServerConfig.device`,
  `DeviceConfig.identity` and `.device(...)` instead of separate identity fields
  or endpoint `.identity(...)`/`.with_identity(...)`. Without a database, full
  servers generate a Device from their configured/default identity; endpoints
  require explicit `.device(...)`. Supplied Devices must match the resolved
  identity. See [migration](docs/rust-api.md#local-receive-capacity-and-outgoing-limits).
