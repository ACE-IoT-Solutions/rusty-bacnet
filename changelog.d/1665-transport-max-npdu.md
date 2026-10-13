---
section: Added
---
- `TransportPort::max_npdu()` reports the longest NPDU a link sends, unicast
  and broadcast, as `NpduLimits`: 1,497 octets each way by default, 501 on
  MS/TP without extended frames, and on BACnet/SC what its hub connection
  carries (#1665).
