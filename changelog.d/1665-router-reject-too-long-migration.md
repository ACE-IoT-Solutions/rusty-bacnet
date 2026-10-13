---
section: Migration notes
---
- **Router egress limits (wire, #1665):** `BACnetRouter` now refuses, with
  reject reason 4, an NPDU longer than its egress port's
  `TransportPort::max_npdu()`, which defaults to 1,497 octets. A custom
  transport that carries longer NPDUs must override `max_npdu()`, or the
  router refuses what it used to pass on.
