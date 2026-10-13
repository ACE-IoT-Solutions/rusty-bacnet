---
section: Added
---
- `BipTransport::receive_counters()` counts the datagrams the B/IP receive
  path drops, by reason: bad headers and payloads, unknown functions,
  destination mismatches, rate-limited and unanswerable requests, untrusted
  forwards, own echoes, a full or closed NPDU channel and socket errors
  (#1622, #1624).
