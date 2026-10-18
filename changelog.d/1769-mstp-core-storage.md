---
section: Changed
---
- **Breaking (Rust API):** `bacnet-mstp-core` takes its buffers as one
  `Storage` bundle (`StandardStorage`, `ExtendedStorage`), `HeaplessQueues`
  is now `FixedQueues`, line inputs return nothing, and `CoreError` splits
  into per-call errors (#1769).
