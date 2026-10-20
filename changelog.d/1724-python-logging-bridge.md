---
section: Added
---
- **Python API:** the stack's log events now reach Python's `logging`, under
  `rusty_bacnet.<module path>` loggers, with the event's fields on each
  record; `refresh_logging()` picks up levels lowered since import (#1724).
