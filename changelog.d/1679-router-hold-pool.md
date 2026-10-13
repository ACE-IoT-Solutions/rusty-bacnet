---
section: Changed
---
- A router's table keeps at most 1,024 reject hold-down records, one per
  network and arrival port, where it kept any number before; a full pool
  gives up its oldest record and counts it in the claim counters'
  `hold_evicted` (#1679).
