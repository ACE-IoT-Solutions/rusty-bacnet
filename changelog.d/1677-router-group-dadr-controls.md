---
section: Changed
---
- **Wire:** `BACnetRouter` drops, counting in `group_dadr_drops()`, a network
  message routed or a reject relayed to a group DADR on a directly connected
  port, and an Unconfirmed-Request for a B/IP group at another UDP port,
  instead of passing it on (#1677).
