---
section: Changed
---
- **Breaking (wire):** Event Enrollment's Event_Type follows Event_Parameters;
  the type given to `EventEnrollmentObject::new` (and Python's
  `add_event_enrollment`) now applies only to legacy octet-string parameters
  (#1868).
