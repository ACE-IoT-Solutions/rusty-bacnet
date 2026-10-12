---
section: Fixed
---
- **Breaking (Rust API):** `MstpCoreConfig::validate` refuses a transmit
  watchdog that wouldn't outlast the longest frame plus T_postdrive, which
  aborted that frame below about 5.1 kbaud; there the baud-rate
  constructors now default to the frame's time plus one second (#1611).
