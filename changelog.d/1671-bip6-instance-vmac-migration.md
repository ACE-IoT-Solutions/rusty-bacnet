---
section: Migration notes
---
- **B/IPv6 VMACs from device instances (Python API, CLI, #1671):** An ipv6
  `BACnetServer`'s VMAC is its device instance, so `start()` fails when
  another node on its link and port uses that instance. A client without
  `ipv6_device_instance` (Python) or `--device-instance` (CLI) picks a random
  VMAC, which needs a multicast-capable link at start: give it an instance on
  Linux loopback or any link without multicast.
