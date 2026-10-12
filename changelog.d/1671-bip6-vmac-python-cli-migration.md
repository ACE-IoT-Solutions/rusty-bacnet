---
section: Migration notes
---
- **B/IPv6 addresses (Python API, CLI, #1671):** Replace `'[ip]:port'` address
  strings with `bip6:<device instance>`, `bip6:0x<6 hex digits>` or the result
  of `await client.resolve_vmac('[ip]:port')`. An ipv6 server's
  `local_address()` now returns `bip6:0x…`. `DiscoveredDevice.mac_address`
  and COV `source_mac`/`source_address` are 3 octets on B/IPv6. Rewrite stored
  18-octet B/IPv6 recipients as the peer's VMAC, and replace
  `encode_bip6_mac`/`decode_bip6_mac` with `Bip6Address`.
