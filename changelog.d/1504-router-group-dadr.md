---
section: Fixed
---
- **Wire:** `BACnetRouter` passes on to a group DADR on a directly
  connected port only an Unconfirmed-Request, and for a B/IP group only at
  the link's own UDP port; it drops anything else, network messages and
  relayed rejects included, counting it in the new `group_dadr_drops()`
  (#1504, #1677).
