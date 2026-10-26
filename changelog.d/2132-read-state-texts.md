---
section: Added
---
- `bacnet read` and `readm` show a binary or multi-state state with the
  object's text, `2 (Low)`, for one extra read an object. JSON `value`
  changes for every such state (`inactive` becomes `inactive (Inactive)`);
  scripts should use `decoded`: a number, or `{value, name}` (#2132).
