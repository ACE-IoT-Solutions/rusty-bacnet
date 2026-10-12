---
section: Changed
---
- **Breaking (Rust API):** The MS/TP transport takes three `Ok(0)` serial
  reads in a row as the port's end of file and stops as at a read error, so
  a custom `SerialPort` must wait for data instead of returning `Ok(0)`; an
  expired late-answer channel now goes at its expiry (#1656).
