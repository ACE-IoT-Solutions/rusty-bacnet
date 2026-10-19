---
section: Added
---
- **Wire:** A registered BBMD's Network Port takes writes to
  BBMD_Broadcast_Distribution_Table and BBMD_Accept_FD_Registrations: checked
  against the BDT policy, pending with Changes_Pending set, and put into effect
  by ReinitializeDevice ACTIVATE_CHANGES or WARMSTART (#1578).
