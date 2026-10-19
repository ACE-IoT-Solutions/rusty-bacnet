---
section: Added
---
- The MS/TP simulator's masters take an application callback that answers
  delivered NPDUs and queues its own sends, replacing `MasterSpec::reply`
  with `application`; the firmware device image's handler now runs on a
  simulated three-master bus (#1796).
