---
section: Changed
---
- **Breaking (Python API):** B/IPv6 peers are named by VMAC: addresses take
  `bip6:<device instance>` or `bip6:0x<6 hex digits>` and refuse `[ip]:port`,
  which `BACnetClient.resolve_vmac` turns into a VMAC. An ipv6 `BACnetServer`'s
  VMAC is its device instance, as `local_address()` shows (#1671).
