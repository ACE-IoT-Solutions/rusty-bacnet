---
section: Migration notes
---
- **B/IP modes (Rust API, CLI, #938):** on `BipEndpointBuilder`,
  `register_as_foreign_device(config)` becomes
  `.mode(BipMode::Foreign(config))`, and `enable_bbmd` with the BBMD setters
  becomes `.mode(BipMode::Bbmd(BbmdOptions { .. }))`; the client and server
  builders take the same `mode`. A `ForeignDeviceConfig` literal adds
  `renewal_interval: None`. The CLI's `register` and `discover --bbmd` take
  the global `--ttl`.
