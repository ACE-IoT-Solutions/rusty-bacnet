---
section: Migration notes
---
- **Group addresses (Rust API, #1622, #1677):** B/IP `send_unicast` to a group
  address fails: broadcast instead. `NetworkLayer::send_apdu` does so for an
  Unconfirmed-Request when the transport gives `group_destinations()` (a
  custom one implements it). The router drops a network message for a group
  DADR. `SendRefusal` and `ForeignRegistrationSnapshot` are
  `#[non_exhaustive]`: match the first with a wildcard arm, and read the
  second's new fields.
