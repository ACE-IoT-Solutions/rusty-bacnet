---
section: Changed
---
- **Breaking (Rust API):** The managed COV helpers take a `CovTarget`, so they
  subscribe to routed devices too and match notifications by source network
  and address; the TUI watch list now subscribes to routed rows instead of
  polling them (#1790).
