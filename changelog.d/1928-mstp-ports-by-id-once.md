---
section: Fixed
---
- On Linux, `bacnet mstp-ports` and the TUI join dialog list a USB serial
  adapter once, by its stable `/dev/serial/by-id` name, instead of also listing
  the `ttyUSB` device it points to (#1928).
