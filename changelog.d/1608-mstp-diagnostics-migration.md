---
section: Migration notes
---
- **MS/TP diagnostics (Rust API, #1608):** for the removed snapshot fields
  read `mac.npdus_delivered` and `mac.frames_received` (`der_rx`, `dner_rx`,
  `reply_postponed_rx`), `mac.parser_errors` (`invalid_frame_discards`) and
  `mac.frame_aborts` (`stale_partial_resets`). `wait_for_reply_timeouts` no
  longer counts waits an invalid frame ended, and `serial_write_errors` also
  counts drain errors; either aborts the frame.
