---
section: Migration notes
---
- **MS/TP codec (Rust API, #1684):** a `FrameSink` returns
  `SinkFull { needed, available }` from `write`, and `FrameError` needs a
  wildcard arm. `crc8`, `crc8_valid`, `crc16`, `crc16_valid` and `crc32k`
  move to `bacnet_mstp_codec::crc`; the `*_accumulate_all` functions and
  `*_RESIDUAL` constants are gone, since the CRC register is the
  complement of what those functions return.
