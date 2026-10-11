---
section: Migration notes
---
- **no_std crates (Rust API, #1604):** a dependency on `bacnet-types`,
  `bacnet-encoding` or `bacnet-services` with `default-features = false` must
  now turn on `alloc` (no_std) or `std`; with neither, the crate refuses to
  build. `serde` on `bacnet-types` turns on `alloc`. `bacnet_types::bip_port`
  and `Error::Transport` need `std`, and a `match` on
  `bacnet_types::error::Error` needs a wildcard arm.
