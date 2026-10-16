---
section: Migration notes
---
- **BACnet runtime:** The first 0.13 runtime slice is intentionally B/IP-only.
  MS/TP, BACnet/SC, router-topology discovery, raw I-Am observation, and COV
  orchestration fail closed until they are ported to current upstream seams;
  routed property targets are also deferred, and the removed 0.12 APIs are not
  emulated with weaker behavior.
