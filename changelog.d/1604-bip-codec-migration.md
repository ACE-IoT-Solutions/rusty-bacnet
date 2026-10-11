---
section: Migration notes
---
- **BVLL (Rust API, #1604):** import `BvlcFunction` and `BvlcResultCode`
  from `bacnet_transport::bvll` or `bacnet_bip_codec`; `SECURE_BVLL` is gone.
  `encode_bvll` takes a `&Bvll` (`encode_bvll_forwarded` is
  `Bvll::ForwardedNpdu`). `BVLC_TYPE_BACNET_IP`, `BVLL_HEADER_LENGTH`,
  `FORWARDED_ADDR_LENGTH` and `bbmd::BDT_ENTRY_SIZE`/`FDT_ENTRY_SIZE` become
  `BVLC_TYPE_BIP`, `HEADER_LENGTH`, `BIP_ADDRESS_LENGTH` and
  `BDT_ENTRY_LENGTH`/`FDT_ENTRY_LENGTH` in `bvll`.
