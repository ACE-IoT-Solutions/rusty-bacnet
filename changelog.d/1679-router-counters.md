---
section: Added
---
- `BACnetRouter::counters()` returns the router engine's counts, such as
  `write_failures`, and `bacnet_network::router` re-exports their `Counters`
  type. Reading them, like the four drop counts, takes no lock (#1679).
