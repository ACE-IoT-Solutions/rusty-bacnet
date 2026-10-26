---
section: Added
---
- `bacnet read` and `readm` show a binary or multi-state state with the
  object's own text, `2 (Low)` or `active (Running)`, at the cost of one
  extra read an object; the JSON `decoded` field keeps the number, and a
  refused text read keeps the plain form (#2132).
