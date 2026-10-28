---
section: Added
---
- **Python API:** `BACnetClient` and `BACnetServer` take
  `sc_control_rate_limit=(burst, per_second)` for the BACnet/SC
  control-message rate limit; Rust SC clients get
  `ScClientBuilder::control_rate_limit` (#1969).
