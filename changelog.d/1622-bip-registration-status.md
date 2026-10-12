---
section: Added
---
- **Breaking (Rust API):** B/IP `ForeignRegistrationSnapshot`, now
  `#[non_exhaustive]`, reports the lease the BBMD's successes prove
  (`registered`, `lease_remaining`), lapses, renewal hints and broadcast
  counts; the transport logs a registration, a run of refusals and a lapse
  once each (#1622).
