---
section: Migration notes
---
- **Router message codes (Rust API, #1684):** `message::code` and
  `message::reason` are the open newtypes `MessageType` and `RejectReason`,
  which `Npdu::message_type`, `Npdu::message`, `Reject::reason`, `Claim`,
  the `Event`s, the `Kind`s and `Table::apply_reject` take. Match their
  constants with a `_` arm, or compare `to_raw()`; `is_proprietary` and
  `is_removed_security` replace the range constants.
