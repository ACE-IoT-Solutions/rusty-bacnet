---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` drops its `heapless` feature
  and its dependency on the `heapless` crate: `HeaplessQueues` keeps its
  slots in arrays and is always available (#1691).
