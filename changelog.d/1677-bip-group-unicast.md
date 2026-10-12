---
section: Changed
---
- **Breaking (wire):** B/IP `send_unicast` to one of the link's group
  addresses (its broadcast, the limited broadcast, a multicast address) fails
  instead of sending a datagram its receivers drop; `BACnetRouter` sends an
  Unconfirmed-Request for such a DADR as that port's local broadcast (#1677).
