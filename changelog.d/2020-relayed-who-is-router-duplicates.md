---
section: Changed
---
- The router's opt-in duplicate suppression also drops repeats of a
  Who-Is-Router it passes on for an unknown network, keyed by the asking
  node, so such a query no longer circulates for good around a loop of
  routers (#2020).
