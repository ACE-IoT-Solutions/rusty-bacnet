---
section: Added
---
- `mstp::monitor::MstpMonitor` listens to an MS/TP trunk without ever
  transmitting and publishes per-station statistics (frame counts, CRC errors,
  token rotation, lost tokens, inferred Max_Master) and a lossy live frame
  stream; `mstp_serial::ReceiveOnlyPort` opens its port (#958).
