---
section: Changed
---
- **Breaking (CLI output):** `bacnet read-range` of an Event Log ends each
  notification's datum with its event values, named for its event type, and
  adds them as typed `event_values` to the record's JSON (#2073).
