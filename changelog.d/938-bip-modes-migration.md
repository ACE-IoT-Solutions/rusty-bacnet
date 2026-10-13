---
section: Migration notes
---
- **B/IP builders (Rust API, CLI, #938):** Replace
  `BipClientBuilder::foreign_device(config)` with
  `.mode(BipMode::Foreign(config))`. On `BipEndpointBuilder`,
  `register_as_foreign_device` becomes `BipMode::Foreign`, and `enable_bbmd`
  with the BBMD setters becomes `.mode(BipMode::Bbmd(BbmdOptions { .. }))`.
  The CLI's `register` and `discover --bbmd` take the global `--ttl`.
