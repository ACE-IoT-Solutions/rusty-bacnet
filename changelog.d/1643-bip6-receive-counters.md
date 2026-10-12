---
section: Added
---
- `Bip6Transport::receive_counters()` counts the datagrams the B/IPv6 receive
  path drops, by reason, and the replies its quotas hold back, as
  `BipTransport::receive_counters()` does for B/IP (#1643).
