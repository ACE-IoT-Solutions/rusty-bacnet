---
section: Changed
---
- The Linux Ethernet transport's AF_PACKET socket goes through socket2 and
  nix with no unsafe code, is close-on-exec, and drops frames queued before
  its bind. A missing interface, or one without a 6-octet MAC (tun,
  WireGuard), fails start with a clear error (#1883).
