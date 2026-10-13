---
section: Fixed
---
- `BACnetRouter` follows a BACnet/SC port's NPDU limit through failover and
  reconnect, published by the new `TransportPort::max_npdu_changes()`, and
  counts a send the transport later refuses as too long in
  `egress_too_long` (#1696).
