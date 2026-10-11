---
section: Fixed
---
- **Breaking (Rust API):** `bacnet-mstp-core` hears the line until a
  transmit start is confirmed instead of taking it for echo, and refuses a
  start within T_turnaround of activity with `CoreError::TransmitDeferred`;
  the frame comes back as a new transmit (#1625).
