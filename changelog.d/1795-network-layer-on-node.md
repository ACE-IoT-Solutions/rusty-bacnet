---
section: Added
---
- `bacnet-router-core` gains `node::Node`, a non-routing node's receive
  rules, sharing the router engine's admission checks and adding
  `Discard::NotRouted`; `NetworkLayer` runs on it, with drop totals in
  `NetworkLayer::counters()`, `malformed` and `routed_drops` included (#1795).
