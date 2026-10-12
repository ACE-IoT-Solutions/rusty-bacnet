---
section: Migration notes
---
- **B/IP group addresses (Rust API, #1622, #1677):** `BipTransport::send_unicast`
  to one of the link's group addresses now fails: send a local broadcast.
  `NetworkLayer::send_apdu` does that itself for an Unconfirmed-Request, and
  `BACnetRouter` drops a network message routed to a group DADR. Match
  `SendRefusal` with a wildcard arm, and read `ForeignRegistrationSnapshot`'s
  new fields rather than building one; both are `#[non_exhaustive]`.
