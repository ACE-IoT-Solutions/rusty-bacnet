---
section: Changed
---
- **Breaking (CLI):** In the TUI audit log a reject or abort outcome's `reason`
  is now a string, the reason's name (or its decimal number when unnamed)
  instead of a number; the result view also names reasons (#1814).
