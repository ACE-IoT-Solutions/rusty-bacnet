---
section: Fixed
---
- The TUI ledger now keeps the old value of an array element it writes, so
  revert puts that element back; when the element couldn't be read first, the
  preview and revert say why (#1801).
