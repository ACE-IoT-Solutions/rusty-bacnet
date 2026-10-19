---
section: Changed
---
- B/IP and B/IPv6 read UDP packet info and receive stamps, and B/IPv6 sends
  its source, through nix's safe socket API on Linux, macOS and the BSDs,
  removing most of bacnet-transport's Unix unsafe code; other targets keep
  the libc path (#1882).
