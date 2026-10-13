---
section: Fixed
---
- **Wire:** `BACnetRouter` no longer removes a learned route when a reject
  with a reason other than 1 or 2 arrives for its network, so a reason-4
  reject can't erase a good route; the reject is still passed on (#1679).
