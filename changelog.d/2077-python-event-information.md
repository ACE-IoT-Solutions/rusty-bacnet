---
section: Changed
---
- **Breaking (Python API):** `get_event_information` returns `{"summaries": [...], "more_events": ...}`
  with named states, `BitString` transitions, timestamps and a new `NotifyType`,
  instead of bytes (#2077).
