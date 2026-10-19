---
section: Changed
---
- B/IP and B/IPv6 read packet info and receive stamps, and send the B/IPv6
  source, through nix's safe socket API on Linux, macOS and the BSDs. Short
  messages nix decodes rely on it trusting the kernel; undecoded ones cost
  an allocation per datagram (#1882).
