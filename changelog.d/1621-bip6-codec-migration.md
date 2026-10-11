---
section: Migration notes
---
- **B/IPv6 BVLL (Rust API, #1621):** import `Bvlc6Function` and
  `Bvlc6ResultCode` from `bacnet_transport::bip6` or `bacnet_bip_codec::ipv6`;
  the transport's enum variants become constants (`ForwardedNpdu` is
  `FORWARDED_NPDU`) and `from_byte`/`to_byte` become `from_raw`/`to_raw`.
  `Bip6Vmac` is a newtype: `Bip6Vmac::new([..])` and `octets()`.
