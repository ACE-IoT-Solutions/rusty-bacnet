---
section: Changed
---
- **Python API:** `atomic_read_file` returns the decoded acknowledgment as a
  dict, and bit strings read, summarized or logged come back as a `BitString`
  with `bits` and, where the type is known, named set bits (#2080).
