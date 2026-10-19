---
section: Added
---
- **Rust API:** `ScClientBuilder::build_with_events` builds a BACnet/SC client
  and returns a hub-connector event receiver subscribed before the start, so
  the first connect is on it (#1890).
