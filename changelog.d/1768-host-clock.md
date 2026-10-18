---
section: Added
---
- `bacnet-time` 0.13.1 adds `earliest`, which merges optional deadlines
  without letting an unset one win as `Option::min` does, and a `std`
  feature with `HostClock`, which maps a host's `std::time::Instant` onto the
  cores' timeline and back. The hosted transports and router use both (#1768).
