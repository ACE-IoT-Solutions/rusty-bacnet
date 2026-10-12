---
section: Changed
---
- **Wire:** `NetworkLayer::send_apdu` sends an Unconfirmed-Request for a group
  destination as the link's local broadcast on every transport that gives a
  group rule, so on Ethernet one for a multicast MAC now goes to the all-ones
  broadcast (#1677).
