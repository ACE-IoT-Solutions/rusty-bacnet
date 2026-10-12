---
section: Fixed
---
- **Wire:** MS/TP: when a successor above Max_Master fails, the master core
  seeks a new one from the address after it, modulo Max_Master + 1, instead
  of from 0 (#1640).
