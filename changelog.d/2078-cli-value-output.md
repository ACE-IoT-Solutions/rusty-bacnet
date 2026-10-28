---
section: Changed
---
- **Breaking (CLI output):** `read`, `readm`, `read-range` and `subscribe`
  show values decoded as the TUI does: enumerations and bits by name,
  schedules, timestamps and recipients rendered, and lists whole. JSON adds
  each value's typed form under `decoded` (#2078).
