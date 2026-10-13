---
section: Fixed
---
- **MS/TP transport:** a port with no drain, such as `TokioSerialPort` on
  Windows, no longer reports a frame finished before its last octet can have
  left: its wire time now counts `MstpConfig::transmit_latency_pad` on top,
  4 ms by default on Windows for a USB adapter (#1654).
