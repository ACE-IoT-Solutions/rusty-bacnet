---
section: Changed
---
- **Breaking (Rust API):** `bacnet-router-core`'s `MstpPort` owns its master
  core, as the B/IP `NodePort` owns its node, and refuses one without
  extended frames (#1769).
