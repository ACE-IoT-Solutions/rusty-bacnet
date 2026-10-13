---
section: Changed
---
- **Wire:** `BACnetRouter` stores an I-Could-Be-Router offer only below its
  256-route cap, as it does learned routes, where it stored one past the
  cap before; one the cap turns away counts in `learned_cap_ignored`
  (#1679).
