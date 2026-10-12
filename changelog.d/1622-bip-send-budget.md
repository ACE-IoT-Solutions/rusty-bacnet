---
section: Changed
---
- **Breaking (wire):** A B/IP transport refuses to send an NPDU longer than
  1497 octets, the most B/IP carries (Table 6-1), with an error, and counts
  it in the new `BipTransport::send_counters()`; it used to send any length
  the BVLL could state (#1622).
