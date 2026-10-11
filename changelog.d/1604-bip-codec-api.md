---
section: Changed
---
- **Breaking (Rust API):** `BvlcFunction` and `BvlcResultCode` move from
  `bacnet-types` to `bacnet-bip-codec`, and `bacnet_transport::bvll` builds
  and reads messages as the codec's typed `Bvll` values (#1604).
