---
section: Changed
---
- **Breaking (CLI):** On `--ipv6`, targets take `bip6:<device instance>` or
  `bip6:0x<6 hex digits>`; an `[ip]` or `[ip]:port` target is first resolved
  to the VMAC of the node there, and addresses print as `bip6:0x…`, which the
  CLI reads back (#1671).
