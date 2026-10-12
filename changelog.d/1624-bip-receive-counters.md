---
section: Added
---
- `BipTransport::receive_counters()` counts the datagrams the B/IP receive
  path drops, by reason: bad headers and payloads, unknown functions,
  destination mismatches, rate-limited requests, own echoes, a full NPDU
  channel and socket errors (#1624).
