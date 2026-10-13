---
section: Migration notes
---
- **MS/TP routing (Rust API, #1611):** a `TransportPort` wrapper forwards
  the new `prepare_for_routing`, which `BACnetRouter` calls to turn on
  extended frames on each MS/TP port it routes, so a router's MS/TP ports
  now declare an APDU limit of 1476.
