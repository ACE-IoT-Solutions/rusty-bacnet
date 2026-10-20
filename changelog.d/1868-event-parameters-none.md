---
section: Fixed
---
- **Wire:** An Event Enrollment whose Event_Parameters were never set serves
  the CHOICE's `none` alternative instead of an empty octet string, and Python
  reads it as `{"kind": "none"}`. Such an enrollment stays out of
  GetEnrollmentSummary (#1868).
