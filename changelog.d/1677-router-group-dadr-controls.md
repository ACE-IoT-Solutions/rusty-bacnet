---
section: Changed
---
- **Wire:** `BACnetRouter` drops a network message routed to a group DADR on
  a directly connected port, counting it in `group_dadr_drops()`, instead of
  passing it on to the group as a unicast (#1677).
