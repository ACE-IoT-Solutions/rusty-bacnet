---
section: Changed
---
- **Breaking (Python API):** `atomic_read_file` and `atomic_write_file`
  return their decoded acknowledgments as dicts, and bit strings read,
  summarized, logged or in `result_flags` come back as a `BitString` with
  `bits` and, where the type is known, named set bits (#2080).
