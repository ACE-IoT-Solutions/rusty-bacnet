---
section: Fixed
---
- **Wire:** An Event Enrollment whose Event_Parameters were never set serves
  the `none` alternative, which Python reads as `{"kind": "none"}`. Any
  enrollment with a monitored reference lists and acknowledges its faults,
  and `none` returns an offnormal one to NORMAL (#1868).
