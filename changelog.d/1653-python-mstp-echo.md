---
section: Added
---
- **Python API:** `BACnetClient` and `BACnetServer` take `mstp_echo`
  (default False) and `mstp_echo_grace_ms` to declare an MS/TP adapter that
  returns what it sends, so the station drops its own echo (#1653).
