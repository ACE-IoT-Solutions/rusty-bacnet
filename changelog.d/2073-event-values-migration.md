---
section: Migration notes
---
- **decode_log_records (Python API, #2073):** a notification's
  `event_values` is a dict whose `kind` names the event type, or `None`, not
  the encoded `bytes`; read its members by name, as docs/python-api.md lists
  them.
