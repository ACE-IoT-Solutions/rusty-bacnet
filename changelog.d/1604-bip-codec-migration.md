---
section: Migration notes
---
- **BVLL (Rust API, #1604):** import `BvlcFunction` and `BvlcResultCode`
  from `bacnet_transport::bvll` or `bacnet_bip_codec`; `SECURE_BVLL` is gone.
  `encode_bvll` takes a `&Bvll` (`encode_bvll_forwarded` is
  `Bvll::ForwardedNpdu`). `BvllMessage` keeps the raw payload: use `npdu()`,
  `origin()` or `message()`. `bbmd::BDT_ENTRY_SIZE`/`FDT_ENTRY_SIZE` become
  the codec's `*_ENTRY_LENGTH`.
