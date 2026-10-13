---
section: Added
---
- **Rust API:** `BipTransport::bvlc_client_snapshot()` reports the
  transport's own BVLC requests and its foreign registration: the lease
  its BBMD's successes prove, the next attempt, lapses, held broadcasts.
  `ForeignDeviceConfig::renewal_interval` sets the renewal, half the TTL by
  default (#1013).
