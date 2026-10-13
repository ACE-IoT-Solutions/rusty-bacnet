---
section: Added
---
- `Bip6Transport::receive_counters()` counts the datagrams the B/IPv6 receive
  path drops, by reason, frames from its own VMAC and from a source no reply
  could reach included, and the replies its quotas hold back, as
  `BipTransport::receive_counters()` does for B/IP (#1637, #1643).
