---
section: Fixed
---
- The TUI ledger now keeps the old value of an array element it writes, so
  revert puts it back, and element and whole writes to one array revert to the
  original; a resize, or an element that couldn't be read, says why (#1801).
