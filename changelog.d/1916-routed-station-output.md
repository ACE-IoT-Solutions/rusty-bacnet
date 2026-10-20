---
section: Changed
---
- **Breaking (CLI):** `discover` and `subscribe` print a routed device's
  station in the target syntax, `2:0x05`, so it can be pasted back as a
  target; the JSON keys are now `station` and `routed_source`, and the
  `Remote MAC` table column is now `Station` (#1916).
