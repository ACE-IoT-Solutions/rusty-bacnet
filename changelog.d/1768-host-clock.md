---
section: Added
---
- `bacnet-time` gains a `std` feature with `HostClock`, which maps a host's
  `std::time::Instant` onto the cores' timeline and back, and `earliest`,
  which merges optional deadlines without letting an unset one win as
  `Option::min` does. The hosted transports and router use both (#1768).
