---
section: Fixed
---
- **Wire:** An Event Enrollment whose Event_Parameters were never set serves
  the CHOICE's `none` alternative instead of an empty octet string, and its
  Event_Type reads NONE; Python reads `{"kind": "none"}`. Such an enrollment
  still reports and lists its faults (#1868).
