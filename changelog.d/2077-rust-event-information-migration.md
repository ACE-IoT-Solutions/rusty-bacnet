---
section: Migration notes
---
- **GetEventInformation (Rust API, #2077):** drop the
  `GetEventInformationAck::decode` call after `get_event_information` or
  `get_event_information_from_device`; they return the decoded ACK, and an ACK
  that does not decode is now their error.
