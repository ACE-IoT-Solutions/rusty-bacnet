---
section: Fixed
---
- The B/IP local-address lookup and the B/IPv6 interface lookup share one
  `getifaddrs` walk, which reads illumos's 64-bit interface flags, so
  `bacnet-transport` builds for illumos (#1884).
