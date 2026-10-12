---
section: Changed
---
- Test-only: the BACnet/SC hub and TLS tests connect again when the OS hands
  an outbound loopback connect a source port that is still in use, as it did
  on Windows CI, instead of failing (#1629).
