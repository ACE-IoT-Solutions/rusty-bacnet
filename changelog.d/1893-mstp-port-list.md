---
section: Added
---
- `bacnet mstp-ports` and the TUI's join dialog list USB serial adapters
  first, once each, by their `/dev/serial/by-id` name on Linux and as
  call-out (`/dev/cu.*`) devices on macOS; `mstp_serial::available_port_infos`
  reports each port's kind (#1893, #1928).
