---
section: Added
---
- **Python API:** `MstpEndpoint` takes the keyword-only
  `mstp_extended_frames`, which lifts its `max_apdu` bound to 1476, and
  `mstp_echo` and `mstp_echo_grace_ms`, with the same rules as on
  `BACnetClient` and `BACnetServer` (#1663).
