---
section: Changed
---
- **Breaking (Rust API):** `MasterNode`, `MasterState` and the `mstp_frame`
  codec are removed; MS/TP frames come from `bacnet_transport::mstp::codec`,
  `SerialPort` gains `echo()`, and `MstpDiagnosticsSnapshot` carries the
  core's counters as `mac` (#1608).
