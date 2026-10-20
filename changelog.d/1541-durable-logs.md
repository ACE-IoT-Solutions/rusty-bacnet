---
section: Added
---
- Trend Log, Trend Log Multiple and Event Log can keep themselves in storage
  (`with_persistence`, `FileLogPersistence`): they commit every 100 records or
  a minute and at stop, and reopen with their records, numbering and
  BUFFER_READY counts plus a LOG_INTERRUPTED record (#1541).
