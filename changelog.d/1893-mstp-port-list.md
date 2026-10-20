---
section: Changed
---
- `bacnet mstp-ports` and the TUI's join dialog list USB serial adapters
  first, and on macOS only call-out (`/dev/cu.*`) devices, without Bluetooth
  ports or the debug console. `mstp_serial::available_port_infos` reports
  each port's kind (#1893).
