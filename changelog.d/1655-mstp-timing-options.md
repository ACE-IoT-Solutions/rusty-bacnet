---
section: Added
---
- **Python API:** `BACnetClient`, `BACnetServer` and `MstpEndpoint` take the
  keyword-only `mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
  `mstp_reply_timeout_ms`, and `MstpEndpointBuilder` the matching setters, for
  slower USB adapters and drivers (#1654, #1655).
