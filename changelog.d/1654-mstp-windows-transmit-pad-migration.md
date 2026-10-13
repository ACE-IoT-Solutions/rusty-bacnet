---
section: Migration notes
---
- **MS/TP transport (Rust API, #1654):** add `transmit_latency_pad`, or
  `..MstpConfig::default()`, to an `MstpConfig` literal that lists every
  field. A custom `SerialPort` without a drain is padded only when the pad is
  set: the default is zero except on Windows. `validate()` refuses a pad over
  15 ms, T_usage_delay.
