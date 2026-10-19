---
section: Changed
---
- The Linux Ethernet transport opens, filters and binds its AF_PACKET socket
  through socket2 and nix with no unsafe code, reads the interface index and
  MAC through nix, sends on the bound socket, and reports a missing interface
  as a clear error before opening a socket (#1883).
