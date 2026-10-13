---
section: Migration notes
---
- **MS/TP transport (Rust API, #1608, #1611, #1654, #1655):** an
  `MstpConfig` literal that lists every field adds `extended_frames`,
  `usage_timeout`, `reply_timeout` and `transmit_latency_pad`, or ends with
  `..MstpConfig::default()`. `start()` and the new `validate()` refuse
  `max_info_frames` 0, baud rates below 2667, timeouts outside 20-35 ms
  (usage) or 255-300 ms (reply), and a pad over 15 ms; 20 and 255 ms keep
  0.12's waits. Declare an adapter that echoes frames with
  `with_echo(Echo::full())`.
