---
section: Added
---
- `mstp::monitor::MstpMonitor` listens to an MS/TP trunk without ever
  transmitting and publishes per-station statistics (frame counts, CRC
  errors, token rotation and hold times, lost tokens, inferred Max_Master)
  and a lossy frame stream; `mstp_serial::ReceiveOnlyPort` opens its port
  (#958, #970).
