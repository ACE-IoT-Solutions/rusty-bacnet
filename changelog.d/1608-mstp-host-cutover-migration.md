---
section: Migration notes
---
- **MS/TP transport (Rust API, #1608, #1611, #1654, #1655):** a full
  `MstpConfig` literal adds `extended_frames`, `usage_timeout`,
  `reply_timeout` and `transmit_latency_pad`, or ends with
  `..MstpConfig::default()`. `start()` and the new `validate()` refuse
  `max_info_frames` 0, baud rates below 2667, timeouts outside 20-35 ms
  (usage) or 255-300 ms (reply), and a pad over 15 ms. 0.12 waited 20 ms
  after a token, 10 after a poll, 255 for a reply. Declare an echoing
  adapter: `with_echo(Echo::full())`.
