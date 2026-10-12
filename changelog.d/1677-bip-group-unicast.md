---
section: Changed
---
- **Breaking (wire):** B/IP `send_unicast` to one of the link's group
  addresses fails instead of sending a datagram its receivers drop;
  `NetworkLayer::send_apdu` and `BACnetRouter` send an Unconfirmed-Request for
  a group address as the link's local broadcast instead (#1677).
