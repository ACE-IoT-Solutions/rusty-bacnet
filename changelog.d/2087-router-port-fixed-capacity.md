---
section: Changed
---
- A router application port keeps the receive capacity it declared when made;
  after a path's limit falls, the router refuses what no longer fits (reject
  reason 4, or a too-long send error), as the docs now say (#2087).
