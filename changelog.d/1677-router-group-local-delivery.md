---
section: Changed
---
- `BACnetRouter` also delivers to its own application an Unconfirmed-Request it
  sends on as a group DADR's local broadcast, as it does a DLEN 0 broadcast
  for the network the request came from (#1677).
