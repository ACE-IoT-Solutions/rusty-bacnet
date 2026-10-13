---
section: Fixed
---
- **Wire:** MS/TP: with a successor above Max_Master, a station's
  maintenance polls end before they reach its own address instead of
  polling itself, and when that successor fails, the search for a new one
  starts from the address after it, modulo Max_Master + 1, not from 0
  (#1640).
