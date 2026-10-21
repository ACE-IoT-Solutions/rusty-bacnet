---
section: Added
---
- `bacnet_transport::warn_throttled!` logs a warning through a `WarnThrottle`:
  at most once per interval with the count since the last one, and at debug
  level otherwise (#1950).
