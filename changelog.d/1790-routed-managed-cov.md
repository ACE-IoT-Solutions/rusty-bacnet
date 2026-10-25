---
section: Changed
---
- **Breaking (Rust API):** The managed COV helpers take a `CovTarget` and
  match notifications by source network and address, and new
  `unsubscribe_cov_target` calls cancel at one; the TUI watch list subscribes
  to routed rows instead of polling them (#1790).
