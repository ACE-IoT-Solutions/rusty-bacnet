---
section: Added
---
- **Python API:** `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take
  `device_write_policy`, a callable that decides writes to the local Device with
  the `MutationContext` and fail-closed rules of `mutation_policy` (#2033).
