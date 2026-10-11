---
section: Changed
---
- **Breaking (Rust API):** `Bvlc6Function` and `Bvlc6ResultCode` move from
  `bacnet-types` to `bacnet-bip-codec`, and `bacnet_transport::bip6` builds
  and reads messages as the codec's typed `Bvll6` values (#1621).
