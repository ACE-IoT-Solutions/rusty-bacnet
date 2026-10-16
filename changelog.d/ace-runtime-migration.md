---
section: Migration notes
---
- **BACnet runtime:** The first 0.13 runtime slice is intentionally B/IP-only.
  MS/TP, BACnet/SC, router-topology discovery, raw I-Am observation, and COV
  orchestration fail closed until they are ported to current upstream seams.
  Explicit property paths may name a directly connected B/IP peer or a remote
  peer through a known B/IP router; automatic route discovery remains deferred,
  and the removed 0.12 APIs are not emulated with weaker behavior.
