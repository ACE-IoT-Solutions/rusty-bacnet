---
section: Changed
---
- B/IP and B/IPv6 packet info, receive stamps and the B/IPv6 send go through
  nix's safe socket API on Linux, macOS and the BSDs; other Unix targets keep
  libc. nix trusts the kernel's message lengths (#1882).
