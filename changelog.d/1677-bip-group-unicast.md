---
section: Changed
---
- **Breaking (wire):** B/IP `send_unicast` to one of the link's group
  addresses fails instead of sending a datagram its receivers drop;
  `NetworkLayer::send_apdu` and `BACnetRouter` send an Unconfirmed-Request for
  one at the link's own UDP port as its local broadcast instead (#1677).
