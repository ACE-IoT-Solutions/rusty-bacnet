---
section: Changed
---
- **Rust API, Python API:** The mutation policy, a Python `mutation_policy`
  callable included, now runs on the blocking pool with no database lock held,
  deciding every WritePropertyMultiple element before any is written, so one
  that blocks no longer stalls other clients or `stop()` (#2032).
