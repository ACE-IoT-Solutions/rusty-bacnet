---
section: Added
---
- **Python API:** the stack's log events now reach Python's `logging` under
  `rusty_bacnet.<crate>` loggers (with a `NullHandler`, so nothing prints
  until the app configures logging); levels are read at import and at each
  start, and `refresh_logging()` rereads them (#1724).
