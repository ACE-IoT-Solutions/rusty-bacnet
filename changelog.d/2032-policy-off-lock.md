---
section: Changed
---
- **Breaking (Rust API):** `ServerPolicy` decisions for received requests,
  a Python `mutation_policy` callable included, run on the blocking pool with
  no lock held, every WritePropertyMultiple element decided first, so a
  blocking policy no longer stalls other clients or `stop()` (#2032).
