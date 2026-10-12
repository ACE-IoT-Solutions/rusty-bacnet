---
section: Changed
---
- **Breaking (wire):** A B/IP transport refuses to send an NPDU longer than
  1497 octets (Table 6-1) with an error, counted in the new
  `BipTransport::send_counters()`; it used to send any length. A BBMD still
  forwards what it receives as it came (#1622).
