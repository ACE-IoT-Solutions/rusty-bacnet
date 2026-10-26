---
section: Changed
---
- **Breaking (Rust API):** `BACnetClient::get_event_information` and its
  `_from_device` form return the decoded `GetEventInformationAck` instead of the
  raw octets, as the new summary calls do (#2077).
