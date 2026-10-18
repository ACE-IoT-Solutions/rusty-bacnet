---
section: Added
---
- The MS/TP core counts broken internal invariants in `internal_errors`
  (the Tokio transport totals them), and its `inspect` feature lets a test
  oracle see a frame before its start (#1769).
