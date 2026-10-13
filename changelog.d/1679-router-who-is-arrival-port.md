---
section: Fixed
---
- **Wire:** `BACnetRouter` sends nothing for any Who-Is-Router-To-Network
  naming a network it reaches through the port the request came in on, which
  the router on that network answers; it used to relay it to its other ports
  (#1679).
