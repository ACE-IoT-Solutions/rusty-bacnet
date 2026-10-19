---
section: Added
---
- `BbmdTables` and `ForeignRegistration` gain `can_change`, false by default:
  a Network Port lists its pending-activation rows writable, and takes writes
  to them, only when its transport's view can apply them (#1825).
