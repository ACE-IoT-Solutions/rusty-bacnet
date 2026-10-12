---
section: Migration notes
---
- **MS/TP transport (Rust API, #1608):** build and read frames with
  `bacnet_transport::mstp::codec` instead of `mstp_frame`; `MstpConfig` is
  unchanged and gains `validate()`. For the removed snapshot fields read
  `mac.npdus_delivered` and `mac.frames_received` (`der_rx`, `dner_rx`,
  `reply_postponed_rx`), `mac.parser_errors` (`invalid_frame_discards`) and
  `mac.frame_aborts` (`stale_partial_resets`). Declare an echoing adapter
  with `with_echo(Echo::Full)`.
