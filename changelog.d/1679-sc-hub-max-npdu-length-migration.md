---
section: Migration notes
---
- **BACnet/SC connection (Rust API, #1679):** Rename
  `ScConnection::hub_max_apdu_length` to `hub_max_npdu_length`; its value is
  unchanged.
