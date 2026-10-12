---
section: Removed
---
- **Breaking (Rust API):** `bacnet_transport::bip6::encode_bip6_mac` and
  `decode_bip6_mac`: a B/IPv6 address is no longer a MAC, so build and read
  one with `Bip6Address` (#1671).
