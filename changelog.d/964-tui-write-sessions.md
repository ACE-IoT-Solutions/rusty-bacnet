---
section: Added
---
- `bacnet tui` can write: time-boxed write sessions armed with `W` (profiles
  default to `writes = "session"`), a preview of every write with
  confirmation tiers, a ledger that reverts every change, array
  elements too, and survives a crash, an exit guard, an audit log, and
  `--passive` (#964, #1801).
