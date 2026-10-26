---
section: Changed
---
- **Breaking (Python API):** `decode_log_records` returns a notification's
  `event_values` as a mapping keyed by member name, with the event type as
  `kind`, instead of the encoded `bytes` (#2073).
