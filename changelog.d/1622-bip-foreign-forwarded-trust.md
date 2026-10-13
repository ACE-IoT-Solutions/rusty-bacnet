---
section: Changed
---
- **Breaking (wire):** A B/IP foreign device takes a Forwarded-NPDU only from
  its configured BBMD by unicast, as B/IPv6 does, and drops any other,
  counting it in `receive_counters()`; normal mode is unchanged (#1622).
