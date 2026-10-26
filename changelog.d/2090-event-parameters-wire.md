---
section: Changed
---
- **Breaking (wire):** An Event Enrollment refuses an Event_Parameters write
  whose command-failure, access-event or other newly decoded algorithm doesn't
  follow the standard's form, which it used to keep as opaque octets (#2090).
