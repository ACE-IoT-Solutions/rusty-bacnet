---
section: Added
---
- **Python API:** Event notification values show per event type, by name:
  `read-range` of an Event Log adds them to each notification, in text and
  as typed `event_values` in JSON, and Python's `decode_log_records` returns
  them as a mapping instead of `bytes` (#2073).
