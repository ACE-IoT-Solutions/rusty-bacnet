---
section: Migration notes
---
- **MS/TP transport (Rust API, #1608):** build and read frames with
  `bacnet_transport::mstp::codec` instead of `mstp_frame`. `start()`, the
  endpoint builder and the new `MstpConfig::validate()` refuse
  `max_info_frames` 0 and baud rates below 2667, accepted before: 40 bit
  times of turnaround must fit in T_usage_delay (15 ms). Declare an adapter
  that hears its own frames with `with_echo(Echo::full())`.
