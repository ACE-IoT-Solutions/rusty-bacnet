---
section: Changed
---
- The TUI keeps at most 4,096 object and device names, forgetting the least
  recently used, and files each under its device's address, so two devices
  sharing an instance keep their own names. A name the device didn't answer
  for shows a dim `?` (#1931).
