---
section: Migration notes
---
- **MS/TP frames (Rust API, #1608, #1684):** `bacnet_transport::mstp_frame`
  is gone: use `bacnet_transport::mstp::codec`, the `bacnet-mstp-codec`
  crate. `FrameType`'s variants are constants (`Token` is `TOKEN`,
  `Unknown(raw)` is `from_raw(raw)`), so a `match` needs a `_` arm.
  `MstpFrame` and `encode_frame` become `Frame::new(..).encode_into`,
  `decode_frame_stream` a `StreamDecoder`, and the CRC functions move to
  `codec::crc`, without the `*_accumulate_all` functions and `*_RESIDUAL`
  constants.
