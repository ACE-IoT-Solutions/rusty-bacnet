---
section: Migration notes
---
- **Group addresses (Rust API, #1677):** B/IP `send_unicast` to a group
  address fails: broadcast instead. `NetworkLayer::send_apdu` does so for an
  Unconfirmed-Request when the transport gives `group_destinations()`, which
  a custom transport implements. The router drops a network message for a
  group DADR.
