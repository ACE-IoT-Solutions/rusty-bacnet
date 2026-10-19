---
section: Migration notes
---
- **Watch CSV (CLI, #1888):** a script reading the TUI watch log should expect
  kebab-case enumerations (`high-limit`, not `HIGH_LIMIT`) and a binary
  value as `active` or `inactive` rather than `1` or `0`; floats keep every
  digit and multi-state values stay numbers.
