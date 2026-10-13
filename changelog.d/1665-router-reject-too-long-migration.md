---
section: Migration notes
---
- **Router egress limits (wire, #1665):** `BACnetRouter` now refuses, with
  reject reason 4, an NPDU longer than its egress port's
  `TransportPort::max_npdu()`, read when the port starts; the default is
  1,497 octets. A custom transport that carries more, or less, must override
  it, and a wrapper around an MS/TP or BACnet/SC transport must delegate it.
  A BACnet/SC port keeps the limit read at start through a failover (#1696).
