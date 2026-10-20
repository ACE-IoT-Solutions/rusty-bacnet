---
section: Migration notes
---
- **Event Enrollment Event_Type (#1868):** set Event_Parameters to choose the
  algorithm; Event_Type follows them. The `event_type` argument of
  `EventEnrollmentObject::new` and Python's `add_event_enrollment(event_type=)`
  now only name the algorithm of legacy octet-string parameters. An
  enrollment whose parameters are never set reads Event_Type NONE.
