---
section: Migration notes
---
- **Device identity (Rust API, #1720):** Pass `bacnet_objects::DeviceIdentity`
  through `.device(...)`, `ServerConfig.device` and `DeviceConfig.identity`.
  Replace the separate vendor, capacity, segmentation and segment-timeout
  settings; endpoint `.identity(...)`/`.with_identity(...)` become `.device(...)`.
  Omit a database to generate a Device after capacity resolution, or supply a
  matching custom Device. See [identity migration](docs/rust-api.md#local-receive-capacity-and-outgoing-limits).
