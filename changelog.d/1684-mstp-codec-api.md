---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-codec`'s `FrameError` is
  `#[non_exhaustive]`, `FrameSink::write` returns a sink-local `SinkFull`,
  the CRC functions move to its `crc` module, and `DecodeEvent` is
  `#[must_use]` (#1684).
