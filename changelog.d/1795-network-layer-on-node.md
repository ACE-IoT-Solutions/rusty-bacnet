---
section: Added
---
- `bacnet-router-core` gains `node::Node`, the receive rules of a node that
  doesn't route, sharing its admission checks with the router engine;
  `NetworkLayer` runs on it and totals its drops in `NetworkLayer::counters()`,
  now with `malformed` and `routed_drops` too (#1795).
